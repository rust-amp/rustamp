// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist editor window: model ownership, input routing, rendering and
//! docked placement (specs **S06**, **S01 §4**, **S05 §§3-5**).
//!
//! The behaviour model is [`rustamp_ui::PlaylistWindow`] (S06) and the layout,
//! hit-testing and frame rendering are [`rustamp_ui::playlist_window`]; this
//! module is the app glue between them. Everything here is headless: the OS
//! window itself is created by [`crate::window`], which calls back into the
//! [`App`] methods below with playlist-window-relative coordinates.
//!
//! Wiring (no behaviour is reimplemented; every gesture below delegates to the
//! model and forwards the command it produces):
//!
//! * list click → [`PlaylistWindow::click_list_first_visible`] (S06 §2: a
//!   single click selects the first visible row);
//! * SEL → [`PlaylistWindow::press_sel`] (S06 §2: selects all visible rows),
//!   REM → [`PlaylistWindow::press_rem`] (S06 §5: removes the selection,
//!   no-op unstaged), ADD → [`PlaylistWindow::press_add`], LIST OPTS →
//!   [`PlaylistWindow::press_list_opts`] whose opening command is
//!   [`Command::OpenPlaylist`] (RFC 0010, S06 §4);
//! * scrollbar arrows → [`PlaylistWindow::scroll_page`] (S06 §3: one step is
//!   one page of four rows), no command;
//! * the title bar has no window-menu button (S06 §1), so a click at (6,3)
//!   does nothing; Close hides the window (`ToggleWindow(Playlist)`, the PL
//!   button reopens it), Shade records a note (the playlist editor's shade is
//!   open, S05 §1.5) and MISC records a note (the `#32768` popup's contents are a
//!   visual reading, S06 §4); ADD and LIST OPTS open their real dialogs
//!   through [`App::request_playlist_dialog`] (the files face of the
//!   open-media dialog for ADD, with the playlist path's unmeasured
//!   `play: false`, RFC 0005 O5; the load-playlist dialog for LIST OPTS,
//!   RFC 0010).
//!
//! Placement: [`App::playlist_window_origin`] replays the main window's shade
//! and double-size state through [`DockedStack`](rustamp_ui::DockedStack), so
//! the playlist follows the main window's bottom edge via the existing S05
//! geometry (equalizer top = main bottom, playlist top = equalizer bottom).
//! The stack positions in the plain double-size state are open (S05 §2.2); the
//! same bottom-edge rule is applied there.

use std::path::PathBuf;

use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_core::playlist::{EntryId, SortKey};
use rustamp_core::state::WindowId;
use rustamp_gfx::Image;
use rustamp_ui::{
    DockedStack, Key, Modifiers, PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistDialog,
    PlaylistTitleButton, ShortcutOutcome, hit_test_button, hit_test_list_row, hit_test_scroll,
    hit_test_title_bar, in_list_area, render_playlist_window,
};

use crate::app::App;

impl App {
    /// The playlist window's client size at rest (S06 scope; S01 §1).
    pub fn playlist_window_size() -> (u32, u32) {
        PLAYLIST_WINDOW_SIZE
    }

    /// Where the playlist window's client origin sits in the docked stack.
    ///
    /// Replays the main window's shade/double-size state through the S05
    /// docked geometry, so the playlist tracks the main window's bottom edge
    /// (through the equalizer's). At rest this is (26,261); with the main
    /// shaded it is (26,159) (S05 §3.3).
    pub fn playlist_window_origin(&self) -> (i32, i32) {
        let mut stack = DockedStack::default_launch();
        let state = self.state();
        stack.set_main_shaded(state.windowshade);
        stack.set_main_double_size(state.double_size);
        (stack.playlist.x, stack.playlist.y)
    }

    /// The playlist behaviour model (the list view the window shows).
    pub fn playlist(&self) -> &rustamp_ui::PlaylistWindow {
        &self.playlist
    }

    /// Syncs the model's entry order from the app's stable entry ids.
    ///
    /// The app is the state owner; selection of live entries and the scroll
    /// offset survive the sync ([`PlaylistWindow::set_entries`]). This runs
    /// whenever the id table grows (an add, a startup file) and before
    /// playlist input is routed, so the model never lags the queue: the
    /// engine queue is the source of truth, the id table mirrors it, and
    /// this mirrors the table.
    pub fn sync_playlist_entries(&mut self) {
        let entries = self.entry_ids.clone();
        self.playlist.set_entries(entries);
    }

    /// A left-button press at playlist-window-relative `(x, y)`.
    ///
    /// Actions fire on the press (the posted-click equivalent of the S06
    /// runs); [`App::playlist_mouse_up`] is the no-op release.
    pub fn playlist_mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        self.sync_playlist_entries();
        if let Some(button) = hit_test_button(x, y) {
            return self.playlist_press(button);
        }
        // S06 §2: a single click selects the first visible row, whatever row
        // was hit, so any point in the band maps to the same model call.
        if in_list_area(x, y) {
            return match self.playlist.click_list_first_visible() {
                Some(command) => {
                    let commands = vec![command.clone()];
                    self.submit(command)?;
                    Ok(commands)
                }
                None => Ok(Vec::new()),
            };
        }
        if let Some(direction) = hit_test_scroll(x, y) {
            self.playlist.scroll_page(direction);
            return Ok(Vec::new());
        }
        if let Some(button) = hit_test_title_bar(x, y) {
            return self.playlist_title_button(button);
        }
        Ok(Vec::new())
    }

    /// A left-button release at playlist-window-relative `(x, y)`.
    ///
    /// Always empty: playlist actions fire on the press (see
    /// [`App::playlist_mouse_down`]).
    pub fn playlist_mouse_up(&mut self, _x: i32, _y: i32) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }

    /// A double-click at playlist-window-relative `(x, y)` (S03 §5, S06 §8 Q5).
    ///
    /// Double-clicking a visible row selects it and starts playback of that
    /// entry ([`Command::PlayEntry`]).
    pub fn playlist_double_click(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        self.sync_playlist_entries();
        if in_list_area(x, y)
            && let Some(row) = hit_test_list_row(x, y)
            && let Some(command) = self.playlist.double_click_row(row)
        {
            let commands = vec![command.clone()];
            self.submit(command)?;
            return Ok(commands);
        }
        Ok(Vec::new())
    }

    /// A key press while the playlist editor is focused (S03 §2).
    ///
    /// The playlist table is consulted first, in this order:
    ///
    /// * Delete (`1034`, remove the selected items, S06 §5);
    /// * Ctrl+S (`40204`, save the playlist: the app records the request and
    ///   the presenter opens the native save dialog);
    /// * the rows [`rustamp_ui::playlist::playlist_key_action`] names — the
    ///   selection moves, jumps and paging (provisional), Enter (`40184`, play
    ///   the selection), `Ctrl+Delete` (`1035`, crop to the selection), the
    ///   four menu-confirmed reorder rows (`Ctrl+Shift+1/2/3` sort by title,
    ///   file name and path and file name, `40209`-`40211`; `Ctrl+Shift+R`
    ///   randomizes the list, `40212`) and the provisional `Alt+Up`/`Alt+Down`
    ///   block moves, which are noted as unimplemented;
    /// * Ctrl+A (`40205`, select all) and Ctrl+I (`40171`, invert the
    ///   selection).
    ///
    /// Every other key takes the shared shortcut path ([`App::key_down`]): S03
    /// leaves how the two tables combine provisional, so a key the playlist
    /// table does not bind keeps whatever global binding it has.
    pub fn playlist_key_down(
        &mut self,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<Option<ShortcutOutcome>> {
        use rustamp_ui::playlist::{PlaylistKeyAction, playlist_key_action};

        self.sync_playlist_entries();

        // Handle Delete first (plain Delete returns None from playlist_key_action)
        if key == Key::Delete && modifiers == Modifiers::NONE {
            match self.playlist.press_rem() {
                Some(command) => {
                    self.submit(command.clone())?;
                    return Ok(Some(ShortcutOutcome::Commands(vec![command])));
                }
                // S06 §5.3: Delete with nothing selected changes nothing
                None => return Ok(Some(ShortcutOutcome::Handled)),
            }
        }

        // Handle Ctrl+S (Save Playlist, command ID 40204, S03 §2)
        // Signal to presenter to open native save-dialog
        if key == Key::Letter(b'S') && modifiers == Modifiers::CTRL {
            self.request_save_playlist = true;
            return Ok(Some(ShortcutOutcome::Handled));
        }

        // Check for playlist-specific key actions (S03 §2)
        if let Some(action) = playlist_key_action(key, modifiers) {
            match action {
                // Navigation actions (provisional, no measured effect)
                PlaylistKeyAction::MoveSelectionUp => {
                    self.playlist.move_selection_up();
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                PlaylistKeyAction::MoveSelectionDown => {
                    self.playlist.move_selection_down();
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                PlaylistKeyAction::JumpToFirst => {
                    self.playlist.jump_to_first();
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                PlaylistKeyAction::JumpToLast => {
                    self.playlist.jump_to_last();
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                PlaylistKeyAction::PageUp => {
                    self.playlist
                        .scroll_page(rustamp_ui::playlist::ScrollDirection::Up);
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                PlaylistKeyAction::PageDown => {
                    self.playlist
                        .scroll_page(rustamp_ui::playlist::ScrollDirection::Down);
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                // Play selected entry (menu-confirmed)
                PlaylistKeyAction::PlaySelected => match self.playlist.play_selected() {
                    Some(command) => {
                        self.submit(command.clone())?;
                        return Ok(Some(ShortcutOutcome::Commands(vec![command])));
                    }
                    None => return Ok(Some(ShortcutOutcome::Handled)),
                },
                // Crop to selected entries (menu-confirmed, Ctrl+Delete)
                PlaylistKeyAction::Crop => {
                    let selected = self.playlist.selected_ids();
                    if selected.is_empty() {
                        return Ok(Some(ShortcutOutcome::Handled));
                    }
                    // Remove all entries that are NOT selected (invert the removal)
                    let selected_set: std::collections::BTreeSet<_> =
                        selected.iter().copied().collect();
                    let ids_to_remove: Vec<EntryId> = self
                        .entry_ids
                        .iter()
                        .copied()
                        .filter(|id| !selected_set.contains(id))
                        .collect();

                    if ids_to_remove.is_empty() {
                        return Ok(Some(ShortcutOutcome::Handled));
                    }

                    let command = Command::Remove(ids_to_remove);
                    self.submit(command.clone())?;
                    self.sync_playlist_entries();
                    return Ok(Some(ShortcutOutcome::Commands(vec![command])));
                }
                // The four reorder rows of S03 §2, menu-confirmed: the app
                // sorts or randomizes its entry-id table and the model
                // re-syncs (`App::sort_entries` / `App::randomize_entries`).
                PlaylistKeyAction::SortByTitle
                | PlaylistKeyAction::SortByFileName
                | PlaylistKeyAction::SortByPathAndFileName => {
                    let key = match action {
                        PlaylistKeyAction::SortByTitle => SortKey::Title,
                        PlaylistKeyAction::SortByFileName => SortKey::FileName,
                        _ => SortKey::PathAndFileName,
                    };
                    let command = Command::Sort(key);
                    self.submit(command.clone())?;
                    return Ok(Some(ShortcutOutcome::Commands(vec![command])));
                }
                PlaylistKeyAction::Randomize => {
                    let command = Command::Randomize;
                    self.submit(command.clone())?;
                    return Ok(Some(ShortcutOutcome::Commands(vec![command])));
                }
                // Block move actions (provisional, not implemented yet)
                PlaylistKeyAction::MoveBlockUp | PlaylistKeyAction::MoveBlockDown => {
                    self.note(format!(
                        "playlist key action {action:?} is not implemented yet (provisional, no Command)"
                    ));
                    return Ok(Some(ShortcutOutcome::Handled));
                }
                // Delete already handled above
                PlaylistKeyAction::Delete => {
                    unreachable!("Delete is handled before playlist_key_action")
                }
            }
        }

        // Legacy shortcuts still handled directly (Ctrl+A, Ctrl+I)
        let command = if key == Key::Letter(b'A') && modifiers == Modifiers::CTRL {
            Some(self.playlist.select_all())
        } else if key == Key::Letter(b'I') && modifiers == Modifiers::CTRL {
            Some(self.playlist.invert_selection())
        } else {
            // Fall through to global shortcuts
            return self.key_down(key, modifiers);
        };

        match command {
            Some(command) => {
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            None => Ok(Some(ShortcutOutcome::Handled)),
        }
    }

    /// Renders one frame of the playlist window.
    pub fn render_playlist(&mut self) -> Image {
        // Playback, not selection, owns Current (S17 section 5). Read the
        // engine on every paint, including natural advancement and no track.
        let index = self.engine().current_index();
        let current = self.entry_id_for(index);
        self.playlist.set_current(current);
        let ids = self.playlist.visible_ids();
        let labels: Vec<String> = ids.iter().map(|id| self.playlist_label(*id)).collect();
        let focused = self.focused_for_playlist();
        render_playlist_window(self.skin.skin(), &self.playlist, &labels, focused)
    }

    /// The row text for one entry: the engine queue's display name at the
    /// entry's index, falling back to the entry id when it has no queue row
    /// (for example after a REM that the engine queue does not track yet).
    fn playlist_label(&self, id: EntryId) -> String {
        match self.entry_ids.iter().position(|entry| *entry == id) {
            Some(index) => self
                .engine()
                .queue()
                .get(index)
                .map(|track| track.display())
                .unwrap_or_else(|| format!("{id:?}")),
            None => format!("{id:?}"),
        }
    }

    /// Which focus variant the playlist frame renders with.
    ///
    /// Simplification, stated: the app tracks one focus flag for the main
    /// window; per-window activation is open, so the playlist shares it.
    fn focused_for_playlist(&self) -> bool {
        self.focused
    }

    /// One bottom-bar button press (S06 §4/§5).
    fn playlist_press(&mut self, button: PlaylistButton) -> Result<Vec<Command>> {
        match button {
            PlaylistButton::Add => {
                let command = self.playlist.press_add();
                self.request_playlist_dialog(PlaylistDialog::AddFiles);
                let commands = vec![command.clone()];
                self.submit(command)?;
                Ok(commands)
            }
            PlaylistButton::ListOpts => {
                let dialog = self.playlist.press_list_opts();
                let command = dialog
                    .opening_command()
                    .expect("LIST OPTS has an opening command (RFC 0010)");
                self.request_playlist_dialog(dialog);
                let commands = vec![command.clone()];
                self.submit(command)?;
                Ok(commands)
            }
            PlaylistButton::Rem => match self.playlist.press_rem() {
                Some(command) => {
                    let commands = vec![command.clone()];
                    self.submit(command)?;
                    Ok(commands)
                }
                // S06 §5.3: without a staged selection the button is a no-op.
                None => Ok(Vec::new()),
            },
            PlaylistButton::Sel => {
                let command = self.playlist.press_sel();
                let commands = vec![command.clone()];
                self.submit(command)?;
                Ok(commands)
            }
            PlaylistButton::Misc => {
                // S06 §4 settles the `#32768` popup's existence and size
                // (474 x 81); its contents are a visual reading and stay
                // open, so the app records the request and opens nothing.
                // S06 §5.2's "no menu stays open" (REM needs no menu choice)
                // holds: this path opens no window either.
                self.note("MISC popup (#32768): the popup is not implemented yet".to_string());
                Ok(Vec::new())
            }
        }
    }

    /// Adds media dropped from the shell onto the playlist editor.
    ///
    /// The same append the ADD button performs: the paths become
    /// [`MediaLocation::File`](rustamp_core::playlist::MediaLocation) items
    /// through the shared [`rustamp_ui::media_items_for_paths`] and are
    /// submitted as one [`Command::AddItems`] with `play: false` (RFC 0005
    /// O5 leaves the playlist add's `play` unmeasured, and the ADD path
    /// already chose `false`), so a drop and a pick cannot drift apart.
    ///
    /// `winit` delivers one `DroppedFile` event per file, so a multi-file
    /// drop arrives as several calls and the entries keep the shell's order.
    /// Dropping is additive: it never replaces the queue, and an empty list
    /// submits nothing.
    ///
    /// A dropped playlist file (`.m3u`, `.pls`, `.xspf`) is resolved by
    /// [`App::add_items`], appending its entries to the queue (own choice:
    /// the reference's drop-a-playlist behaviour is unmeasured, S06 leaves it
    /// open).
    pub fn drop_on_playlist(&mut self, paths: &[PathBuf]) -> Result<Vec<Command>> {
        let items = rustamp_ui::media_items_for_paths(paths);
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let command = Command::AddItems {
            items,
            at: None,
            play: false,
        };
        self.submit(command.clone())?;
        self.sync_playlist_entries();
        Ok(vec![command])
    }

    /// One title-bar button press (S01 §4 rects, provisional).
    ///
    /// Close hides the playlist editor: it is a toggle window, and closing a
    /// toggle window hides it (RFC 0005 `CloseFocusedWindow`; S03 §2 Ctrl+F4
    /// "Close the playlist editor"), the same `ToggleWindow` the main
    /// window's PL button and the menu row send, so all three agree and the
    /// PL button reopens it. Shade records a note and changes nothing: the
    /// playlist editor's windowshade size and drawing are open (S05 §1.5).
    /// A (6,3) click never reaches here ([`hit_test_title_bar`] returns
    /// `None`, S06 §1).
    fn playlist_title_button(&mut self, button: PlaylistTitleButton) -> Result<Vec<Command>> {
        match button {
            PlaylistTitleButton::Close => self.close_playlist_window(),
            PlaylistTitleButton::Shade => {
                self.note(
                    "playlist Shade button: the playlist editor's windowshade is open \
                     (S05 §1.5), so nothing changes"
                        .to_string(),
                );
                Ok(Vec::new())
            }
        }
    }

    /// Hides the playlist editor when it is open (its title-bar Close and its
    /// OS close box). Returns the command it sent, if any.
    pub fn close_playlist_window(&mut self) -> Result<Vec<Command>> {
        if !self.playlist_open() {
            return Ok(Vec::new());
        }
        let command = Command::ToggleWindow(WindowId::Playlist);
        self.submit(command.clone())?;
        Ok(vec![command])
    }
}

#[cfg(test)]
mod tests {
    use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
    use rustamp_core::config::Config;
    use rustamp_core::playlist::{EntryId, MediaLocation};
    use rustamp_core::state::WindowId;

    use super::*;
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: None,
            seconds: None,
        };
        App::new(options).expect("the test app builds headless")
    }

    /// Grows the engine queue (no decoding: [`Engine::set_queue`] only stores)
    /// so the app assigns stable entry ids for the playlist to show.
    fn add_entries(app: &mut App, n: u32) {
        let items = (0..n)
            .map(|i| MediaLocation::File(PathBuf::from(format!("track{i}.mp3"))))
            .collect::<Vec<_>>();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .expect("AddItems applies");
    }

    /// Check actual row ink through the app's production render path, without
    /// golden files or a desktop. None must overwrite any previous marker.
    fn assert_current_frame(app: &mut App, current: Option<EntryId>) {
        let frame = app.render_playlist();
        assert_eq!(app.playlist().current(), current);
        let style = app.skin.skin().pledit();
        assert_ne!(style.normal, style.current);
        let selected = app.playlist().selected_ids();
        for (row, id) in app.playlist().visible_ids().iter().enumerate() {
            let rect = rustamp_ui::playlist_window::row_rect(row).unwrap();
            let ink = if current == Some(*id) {
                style.current
            } else {
                style.normal
            };
            let bg = if selected.contains(id) {
                style.selected_bg
            } else {
                style.normal_bg
            };
            let ink = [ink[0], ink[1], ink[2], 255];
            let bg = [bg[0], bg[1], bg[2], 255];
            let mut ink_count = 0;
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let pixel = frame.pixel(x, y).unwrap();
                    assert!(pixel == ink || pixel == bg, "row {row}: {pixel:?}");
                    ink_count += usize::from(pixel == ink);
                }
            }
            assert!(ink_count > 0, "the queue label must render in row {row}");
        }
    }

    #[test]
    fn current_row_ink_follows_the_engine_without_selecting_or_scrolling() {
        let mut app = test_app();
        assert_current_frame(&mut app, None);
        add_entries(&mut app, 8);
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(0)])))
            .unwrap();
        // A populated, selected playlist is not evidence of playback. There
        // is no decoded metadata; labels still come from each queue source.
        assert_eq!(app.engine().current_index(), None);
        assert_current_frame(&mut app, None);
        for index in [Some(1), Some(0), Some(5), None] {
            app.engine_mut().set_current_index(index);
            assert_current_frame(&mut app, index.map(|i| EntryId(u64::from(i))));
            assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
            assert_eq!(app.playlist().first_visible(), 0);
        }
        app.playlist.scroll_to(4);
        for index in [Some(5), Some(1), None, Some(99)] {
            app.engine_mut().set_current_index(index);
            let current = index.filter(|i| *i < 8).map(|i| EntryId(u64::from(i)));
            assert_current_frame(&mut app, current);
            assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
            assert_eq!(app.playlist().first_visible(), 4);
        }
    }

    #[test]
    fn an_empty_queue_label_does_not_reuse_the_previous_title() {
        let mut app = test_app();
        add_entries(&mut app, 1);
        app.engine_mut().set_current_index(Some(0));
        assert_current_frame(&mut app, Some(EntryId(0)));
        // No playback or metadata: directly exercise the existing empty-label
        // renderer contract after a frame that had visible title ink.
        app.engine_mut()
            .set_queue(vec![rustamp_audio::TrackSource::Url(String::new())]);
        let frame = app.render_playlist();
        assert_eq!(app.playlist().current(), Some(EntryId(0)));
        let rect = rustamp_ui::playlist_window::row_rect(0).unwrap();
        let bg = app.skin.skin().pledit().normal_bg;
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                assert_eq!(frame.pixel(x, y), Some([bg[0], bg[1], bg[2], 255]));
            }
        }
    }

    #[test]
    fn queue_edits_never_lend_a_stale_current_index_to_another_entry() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.engine_mut().set_current_index(Some(2));
        assert_current_frame(&mut app, Some(EntryId(2)));
        // Removing an earlier row moves the index, not the current identity.
        app.submit(Command::Remove(vec![EntryId(0)])).unwrap();
        assert_eq!(app.engine().current_index(), Some(1));
        assert_current_frame(&mut app, Some(EntryId(2)));
        // Entry 3 now takes index 1 but must not inherit Current.
        app.submit(Command::Remove(vec![EntryId(2)])).unwrap();
        assert_eq!(app.engine().current_index(), None);
        assert_current_frame(&mut app, None);
        app.engine_mut().set_current_index(Some(0));
        assert_current_frame(&mut app, Some(EntryId(1)));
        app.submit(Command::Clear).unwrap();
        assert_current_frame(&mut app, None);
        add_entries(&mut app, 2);
        assert_eq!(app.engine().current_index(), None);
        assert_current_frame(&mut app, None);
    }

    #[test]
    fn replacing_the_playlist_without_playing_clears_current_and_old_ids() {
        let mut app = test_app();
        add_entries(&mut app, 2);
        app.engine_mut().set_current_index(Some(1));
        assert_current_frame(&mut app, Some(EntryId(1)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replacement.m3u");
        std::fs::write(&path, "#EXTM3U\nreplacement0.mp3\nreplacement1.mp3\n").unwrap();
        app.load_playlist_file(&path, false).unwrap();
        assert_eq!(app.engine().current_index(), None);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(2), EntryId(3)]);
        assert_current_frame(&mut app, None);
    }

    /// The drag-and-drop defect: dropping files on the playlist window did
    /// nothing, because only the dialog window had a drop handler.
    ///
    /// The model half is checked here: a drop appends the files, in order,
    /// without replacing what is there, and an empty drop changes nothing.
    /// That the OS delivers the drop to the playlist window needs a desktop
    /// and a shell drag, so only the owner's machine can confirm it.
    #[test]
    fn dropping_files_on_the_playlist_appends_them_in_order() {
        let mut app = test_app();
        add_entries(&mut app, 1);
        app.sync_playlist_entries();
        let before = app.playlist().len();
        let dropped = [
            PathBuf::from("C:/Music/first.mp3"),
            PathBuf::from("C:/Music/second.mp3"),
        ];
        let commands = app.drop_on_playlist(&dropped).expect("the drop applies");
        assert_eq!(
            commands.len(),
            1,
            "one AddItems for the batch: {commands:?}"
        );
        assert!(matches!(
            commands[0],
            Command::AddItems {
                play: false,
                at: None,
                ..
            }
        ));
        assert_eq!(
            app.playlist().len(),
            before + 2,
            "a drop appends and never replaces"
        );
        let queue = app.engine().queue();
        assert!(
            queue
                .get(queue.len() - 2)
                .expect("the first dropped entry")
                .display()
                .contains("first"),
            "the shell's order is kept"
        );
        assert!(
            queue
                .last()
                .expect("the second dropped entry")
                .display()
                .contains("second")
        );
        // Nothing dropped changes nothing.
        let unchanged = app.playlist().len();
        assert!(app.drop_on_playlist(&[]).expect("an empty drop").is_empty());
        assert_eq!(app.playlist().len(), unchanged);
    }

    #[test]
    fn playlist_window_is_275x116_at_rest() {
        // S06 scope / S01 §1, verified; S05 §2.1 leaves it unchanged.
        assert_eq!(App::playlist_window_size(), (275, 116));
        assert_eq!(App::playlist_window_size(), PLAYLIST_WINDOW_SIZE);
    }

    #[test]
    fn docked_origin_follows_the_main_bottom_edge() {
        // S05 §3.1/§3.3: rest (26,261), shaded (26,159), then back.
        let mut app = test_app();
        assert_eq!(app.playlist_window_origin(), (26, 261));
        app.submit(Command::ToggleWindowShade(WindowId::Main))
            .expect("shade toggles");
        assert_eq!(app.playlist_window_origin(), (26, 159));
        app.submit(Command::ToggleWindowShade(WindowId::Main))
            .expect("unshade toggles");
        assert_eq!(app.playlist_window_origin(), (26, 261));
    }

    #[test]
    fn a_list_click_selects_the_first_visible_row() {
        // S06 §2: single click selects the first visible row.
        let mut app = test_app();
        add_entries(&mut app, 60);
        let commands = app.playlist_mouse_down(60, 28).expect("click applies");
        assert_eq!(
            commands,
            vec![Command::Select(SelectOp::Set(vec![EntryId(0)]))]
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
    }

    #[test]
    fn sel_selects_all_visible_rows() {
        // S06 §2: SEL selects all visible rows.
        let mut app = test_app();
        add_entries(&mut app, 60);
        let commands = app.playlist_mouse_down(74, 92).expect("SEL applies");
        let all = vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)];
        assert_eq!(commands, vec![Command::Select(SelectOp::Set(all.clone()))]);
        assert_eq!(app.playlist().selected_ids(), all);
    }

    #[test]
    fn rem_removes_the_staged_selection() {
        // S06 §5: REM removes the selected entry with no menu choice.
        let mut app = test_app();
        add_entries(&mut app, 60);
        app.playlist_mouse_down(60, 28).expect("stage a selection");
        let commands = app.playlist_mouse_down(46, 92).expect("REM applies");
        assert_eq!(commands, vec![Command::Remove(vec![EntryId(0)])]);
        assert_eq!(app.playlist().len(), 59);
        assert_eq!(app.engine().queue().len(), 59);
        assert_eq!(app.entry_ids.len(), 59);
        assert!(app.playlist().selected_ids().is_empty());
    }

    #[test]
    fn double_clicking_a_row_emits_play_entry() {
        // S03 §5, S06 §8 Q5: double-clicking a playlist entry plays it.
        let mut app = test_app();
        add_entries(&mut app, 6);
        // Row 1 (LIST_TOP + 13 + 6 = 42, x = 60 in list band)
        let commands = app
            .playlist_double_click(60, 42)
            .expect("double click applies");
        assert_eq!(commands, vec![Command::PlayEntry(EntryId(1))]);
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);
    }

    #[test]
    fn rem_without_a_staged_selection_is_a_no_op() {
        // S06 §5.3: 0 px with nothing staged.
        let mut app = test_app();
        add_entries(&mut app, 60);
        let commands = app.playlist_mouse_down(46, 92).expect("REM applies");
        assert!(commands.is_empty());
        assert_eq!(app.playlist().len(), 60);
    }

    #[test]
    fn add_and_list_opts_emit_their_commands_with_their_titles() {
        // S06 §4 (titles) + RFC 0010 (LIST OPTS maps to OpenPlaylist). ADD
        // maps to the files face of the open-media dialog (RFC 0005) with the
        // playlist path's unmeasured `play: false` (O5, kept as sent); both
        // buttons open a real dialog now, so neither leaves a note.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let commands = app.playlist_mouse_down(24, 92).expect("ADD applies");
        assert_eq!(
            commands,
            vec![Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            }]
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        let commands = app.playlist_mouse_down(242, 92).expect("LIST OPTS applies");
        assert_eq!(commands, vec![Command::OpenPlaylist]);
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn confirming_the_add_dialog_appends_without_a_note() {
        // The full playlist path: ADD opens the files face, a staged pick
        // confirms through `AddItems`, the dialog closes and the model grows.
        // `play: false` rides through untouched (RFC 0005 O5 stays open).
        let mut app = test_app();
        add_entries(&mut app, 3);
        app.playlist_mouse_down(24, 92).expect("ADD applies");
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        // The engine queue is the source of truth for the add; the id-table
        // growth re-syncs the playlist model at once, with no button press.
        assert_eq!(app.engine().queue().len(), 4);
        assert_eq!(app.playlist().len(), 4);
        assert_eq!(
            app.playlist().visible_ids(),
            vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)]
        );
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn adding_items_syncs_the_model_with_no_button_press() {
        // The source-of-truth path: `AddItems` grows the engine queue, the
        // id table and the playlist model together (no dialog, no click).
        let mut app = test_app();
        assert!(app.playlist().is_empty());
        add_entries(&mut app, 2);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(0), EntryId(1)]);
        // An empty add changes nothing and stays silent.
        app.submit(Command::AddItems {
            items: Vec::new(),
            at: None,
            play: false,
        })
        .expect("empty AddItems applies");
        assert_eq!(app.playlist().len(), 2);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn playlist_sync_is_idempotent_and_keeps_the_selection() {
        // Re-syncing changes nothing: same entries, same selection, same view.
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28).expect("stage a selection");
        let selected = app.playlist().selected_ids();
        assert_eq!(selected, vec![EntryId(0)]);
        app.sync_playlist_entries();
        app.sync_playlist_entries();
        assert_eq!(app.playlist().len(), 6);
        assert_eq!(app.playlist().selected_ids(), selected);
        assert_eq!(app.playlist().first_visible(), 0);
    }

    #[test]
    fn an_empty_queue_syncs_to_an_empty_model() {
        // A fresh app shows nothing; syncing the empty table is a no-op.
        let mut app = test_app();
        assert_eq!(app.engine().queue().len(), 0);
        assert!(app.playlist().is_empty());
        assert!(app.playlist().visible_ids().is_empty());
        app.sync_playlist_entries();
        assert!(app.playlist().is_empty());
        assert!(app.playlist().selected_ids().is_empty());
    }

    #[test]
    fn misc_records_a_note_and_opens_nothing() {
        // S06 §4 settles the popup's existence; its contents stay open.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let commands = app.playlist_mouse_down(112, 92).expect("MISC applies");
        assert!(commands.is_empty());
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("MISC")), "{notes:?}");
    }

    #[test]
    fn scrollbar_arrows_page_through_the_list() {
        // S06 §3: one arrow step is one page of four rows.
        let mut app = test_app();
        add_entries(&mut app, 60);
        app.playlist_mouse_down(263, 87).expect("down applies");
        assert_eq!(app.playlist().first_visible(), 4);
        app.playlist_mouse_down(263, 81).expect("up applies");
        assert_eq!(app.playlist().first_visible(), 0);
    }

    #[test]
    fn the_title_bar_has_no_window_menu_button() {
        // S06 §1: a (6,3) click does nothing.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let commands = app.playlist_mouse_down(6, 3).expect("click applies");
        assert!(commands.is_empty());
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn the_title_bar_close_hides_the_editor_and_pl_reopens_it() {
        // BETA-06: the playlist's own Close hides it (a toggle window, RFC
        // 0005 `CloseFocusedWindow`) without quitting; the main window's PL
        // toggle brings it back. The editor is open at launch.
        let mut app = test_app();
        assert!(app.playlist_open());
        let commands = app.playlist_mouse_down(264, 3).expect("close applies");
        assert_eq!(commands, vec![Command::ToggleWindow(WindowId::Playlist)]);
        assert!(!app.playlist_open());
        assert!(!app.quit());
        assert!(app.drain_notes().is_empty());
        // Closing an already hidden editor sends nothing.
        assert!(app.close_playlist_window().expect("no-op").is_empty());
        app.submit(Command::ToggleWindow(WindowId::Playlist))
            .expect("PL toggles");
        assert!(app.playlist_open());
    }

    #[test]
    fn the_title_bar_shade_says_why_it_does_nothing() {
        // S05 §1.5: the playlist editor's windowshade is open.
        let mut app = test_app();
        let commands = app.playlist_mouse_down(254, 3).expect("shade applies");
        assert!(commands.is_empty());
        assert!(app.playlist_open());
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("S05 §1.5")), "{notes:?}");
    }

    #[test]
    fn rendering_produces_a_frame_of_the_playlist_size() {
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28).expect("stage a selection");
        let image = app.render_playlist();
        assert_eq!((image.width(), image.height()), (275, 116));
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0),
            "the frame is empty"
        );
    }

    #[test]
    fn delete_removes_the_selected_entry_in_the_playlist_window() {
        // S03 §2: Delete (`1034`, menu) removes the selected items (S06 §5).
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28).expect("stage a selection");
        let outcome = app
            .playlist_key_down(Key::Delete, Modifiers::NONE)
            .expect("Delete applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Remove(vec![
                EntryId(0)
            ])]))
        );
        assert_eq!(app.playlist().len(), 5);
        assert_eq!(app.engine().queue().len(), 5);
        assert_eq!(app.entry_ids.len(), 5);
    }

    #[test]
    fn delete_without_a_selection_is_handled_and_changes_nothing() {
        // S06 §5.3: nothing staged, nothing removed.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let outcome = app
            .playlist_key_down(Key::Delete, Modifiers::NONE)
            .expect("Delete applies");
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert_eq!(app.playlist().len(), 3);
    }

    #[test]
    fn ctrl_a_selects_all_and_ctrl_i_inverts_in_the_playlist_window() {
        // S03 §2: Ctrl+A (`40205`) and Ctrl+I (`40171`), both menu-confirmed.
        let mut app = test_app();
        add_entries(&mut app, 4);
        let outcome = app
            .playlist_key_down(Key::Letter(b'A'), Modifiers::CTRL)
            .expect("Ctrl+A applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Select(
                SelectOp::All
            )]))
        );
        assert_eq!(app.playlist().selected_ids().len(), 4);
        app.playlist_key_down(Key::Letter(b'I'), Modifiers::CTRL)
            .expect("Ctrl+I applies");
        assert!(app.playlist().selected_ids().is_empty());
    }

    #[test]
    fn other_playlist_keys_take_the_shared_path_and_leave_the_list_alone() {
        // S03 §2: Up is a provisional playlist row with no measured effect, so
        // it reaches the shared shortcut path and does not move the selection.
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28).expect("stage a selection");
        app.playlist_key_down(Key::Up, Modifiers::NONE)
            .expect("Up applies");
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
        assert_eq!(app.playlist().len(), 6);
    }

    #[test]
    fn ctrl_shift_chords_sort_and_randomize_the_playlist() {
        // S03 §2: `40209`-`40212` — Ctrl+Shift+1/2/3 sort by title, file name
        // and path and file name, Ctrl+Shift+R randomizes the list; all four
        // are menu-confirmed commands.
        let mut app = test_app();
        let items = vec![
            MediaLocation::File(PathBuf::from("zulu.mp3")),
            MediaLocation::File(PathBuf::from("alpha.mp3")),
            MediaLocation::File(PathBuf::from("mike.mp3")),
        ];
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        let chord = Modifiers::CTRL | Modifiers::SHIFT;

        let outcome = app
            .playlist_key_down(Key::Digit(2), chord)
            .expect("Ctrl+Shift+2 applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Sort(
                SortKey::FileName
            )]))
        );
        // alpha(1), mike(2), zulu(0): model and id table move together.
        assert_eq!(
            app.playlist().visible_ids(),
            vec![EntryId(1), EntryId(2), EntryId(0)]
        );

        // Without a loaded report every title falls back to the file name, so
        // Ctrl+Shift+1 keeps that order; Ctrl+Shift+3 sorts the same names.
        for (digit, key) in [
            (Key::Digit(1), SortKey::Title),
            (Key::Digit(3), SortKey::PathAndFileName),
        ] {
            let outcome = app
                .playlist_key_down(digit, chord)
                .expect("the chord applies");
            assert_eq!(
                outcome,
                Some(ShortcutOutcome::Commands(vec![Command::Sort(key)]))
            );
        }
        assert_eq!(
            app.playlist().visible_ids(),
            vec![EntryId(1), EntryId(2), EntryId(0)]
        );

        let outcome = app
            .playlist_key_down(Key::Letter(b'R'), chord)
            .expect("Ctrl+Shift+R applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Randomize]))
        );
        // The entries survive the shuffle; only their rows move.
        let mut ids = app.playlist().visible_ids();
        ids.sort_by_key(|id| id.0);
        assert_eq!(ids, vec![EntryId(0), EntryId(1), EntryId(2)]);

        // A Ctrl+Shift chord the table does not bind keeps the shared path.
        assert_eq!(
            app.playlist_key_down(Key::Digit(4), chord)
                .expect("an unbound key"),
            None
        );
    }

    /// Opening the playlist window needs an OS window, so this never runs in
    /// the headless suite: it pins the creation parameters the real window
    /// must carry (275 x 116 client area, docked below the main window) for
    /// the test-VM run that does open it.
    #[ignore = "opens an OS window; run manually inside the headless test VM only"]
    #[test]
    fn opening_the_playlist_window_needs_an_os_window() {
        assert_eq!(App::playlist_window_size(), (275, 116));
        assert_eq!(test_app().playlist_window_origin(), (26, 261));
    }
}
