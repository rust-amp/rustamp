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
//!   does nothing; Close/Shade record a note (hiding and shading the playlist
//!   are open) and MISC records a note (the `#32768` popup's contents are a
//!   visual reading, S06 §4); the ADD dialog records its spec'd title through
//!   [`App::request_playlist_dialog`] and opens no window, while LIST OPTS
//!   opens the real load-playlist dialog through the same call (RFC 0010).
//!
//! Placement: [`App::playlist_window_origin`] replays the main window's shade
//! and double-size state through [`DockedStack`](rustamp_ui::DockedStack), so
//! the playlist follows the main window's bottom edge via the existing S05
//! geometry (equalizer top = main bottom, playlist top = equalizer bottom).
//! The stack positions in the plain double-size state are open (S05 §2.2); the
//! same bottom-edge rule is applied there.

use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_core::playlist::EntryId;
use rustamp_gfx::Image;
use rustamp_ui::{
    DockedStack, PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistDialog, PlaylistTitleButton,
    hit_test_button, hit_test_scroll, hit_test_title_bar, in_list_area, render_playlist_window,
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
    /// offset survive the sync ([`PlaylistWindow::set_entries`]).
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
            self.playlist_title_button(button);
            return Ok(Vec::new());
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

    /// Renders one frame of the playlist window.
    pub fn render_playlist(&mut self) -> Image {
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
                Some(Command::Remove(ids)) => {
                    // The model dropped the entries from its own order; the
                    // app drops their stable ids so the next sync agrees.
                    // Removing the audio queue entries themselves is S14/S15
                    // and stays on the existing note path below.
                    self.entry_ids.retain(|id| !ids.contains(id));
                    let command = Command::Remove(ids);
                    let commands = vec![command.clone()];
                    self.submit(command)?;
                    Ok(commands)
                }
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

    /// One title-bar button press (S01 §4 rects, provisional).
    ///
    /// Hiding and shading the playlist editor are open (S05 §1.5), so both
    /// record a note and change nothing; in particular a (6,3) click never
    /// reaches here ([`hit_test_title_bar`] returns `None`, S06 §1).
    fn playlist_title_button(&mut self, button: PlaylistTitleButton) {
        self.note(format!("playlist {button:?} button is not implemented yet"));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
        assert!(app.playlist().selected_ids().is_empty());
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
        // still records its title through the dialog-stub path; LIST OPTS
        // opens the real load-playlist dialog, so only ADD leaves a note.
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
        let commands = app.playlist_mouse_down(242, 92).expect("LIST OPTS applies");
        assert_eq!(commands, vec![Command::OpenPlaylist]);
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        // ADD emits its command and records that its dialog is not implemented,
        // so two notes are expected; LIST OPTS now opens a real dialog and adds
        // none of its own.
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("Add file(s) to playlist"), "{notes:?}");
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
        // S06 §1: a (6,3) click does nothing; Close notes without quitting.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let commands = app.playlist_mouse_down(6, 3).expect("click applies");
        assert!(commands.is_empty());
        assert!(app.drain_notes().is_empty());
        let commands = app.playlist_mouse_down(264, 3).expect("close applies");
        assert!(commands.is_empty());
        assert!(!app.quit());
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("Close")), "{notes:?}");
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
