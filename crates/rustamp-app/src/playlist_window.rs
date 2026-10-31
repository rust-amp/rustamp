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
//! * list press/move/release → [`PlaylistWindow::mouse_down`],
//!   [`PlaylistWindow::mouse_move`], [`PlaylistWindow::mouse_up`]. S06 §2
//!   settles the first-row click; the selection gestures around it (the
//!   shift-range, the ctrl-toggle) and the drag rules are own choices
//!   (S06 §8 Q6/Q11/Q14);
//! * `Shift+Up`/`Shift+Down` → [`PlaylistWindow::extend_selection`], the
//!   keyboard face of the same range (own choice: S03 §2 binds neither
//!   chord);
//! * SEL → [`PlaylistWindow::press_sel`] (S06 §2: selects all visible rows),
//!   REM → [`PlaylistWindow::press_rem`] (S06 §5: removes the selection,
//!   no-op unstaged), ADD → [`PlaylistWindow::press_add`], LIST OPTS →
//!   [`PlaylistWindow::press_list_opts`] whose opening command is
//!   [`Command::OpenPlaylist`] (RFC 0010, S06 §4);
//! * the scrollbar → [`PlaylistWindow::press_scrollbar`]: the arrow pair
//!   pages the view (S06 §3: one step is one page of four rows), the handle
//!   takes hold of it and a drag moves the view, not a selection (S06 §3),
//!   and a press on the track pages the view (own choice); no command;
//! * the title bar has no window-menu button (S06 §1), so a click at (6,3)
//!   does nothing; Close hides the window (`ToggleWindow(Playlist)`, the PL
//!   button reopens it), Shade toggles the editor's windowshade
//!   (`ToggleWindowShade(Playlist)`: the editor collapses to its 14 px title
//!   strip and expands again, S01 §1) and MISC requests the native popup
//!   (S04 §7);
//!   ADD and LIST OPTS open their real dialogs
//!   through [`App::request_playlist_dialog`] (the files face of the
//!   open-media dialog for ADD, with the playlist path's unmeasured
//!   `play: false`, RFC 0005 O5; the load-playlist dialog for LIST OPTS,
//!   RFC 0010).
//!
//! Placement: [`App::docked_stack`] replays the windows' shade and
//! double-size state through [`DockedStack`](rustamp_ui::DockedStack), so the
//! playlist follows the main window's bottom edge via the existing S05
//! geometry (equalizer top = main bottom, playlist top = equalizer bottom) and
//! a shaded editor is one row tall (S01 §1). The stack positions in the plain
//! double-size state are open (S05 §2.2); the same bottom-edge rule is applied
//! there.

use std::path::PathBuf;

use rustamp_audio::TrackSource;
use rustamp_core::command::{Command, OpenMediaKind};
use rustamp_core::error::Result;
use rustamp_core::playlist::{EntryId, SortKey};
use rustamp_core::state::WindowId;
use rustamp_gfx::Image;
use rustamp_ui::title_format::TitleFormat;
use rustamp_ui::{
    DockedStack, Key, Modifiers, PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistDialog,
    PlaylistKeyAction, PlaylistTitleButton, ShortcutOutcome, WindowKind, hit_test_button,
    hit_test_title_bar, render_playlist_window, window_size,
};

use crate::app::App;
use crate::title_format::TitleFormatSetting;

/// A right click on a playlist entry asking for that entry's **context menu**
/// (S03 §5).
///
/// S03 §5 measures the gesture — a right click on an entry opens the
/// playlist's context menu for it — and the menu itself is **blocked on
/// S04**: its rows, wording and enabled states are S04's, not this build's
/// to invent. This type therefore names only what the click settled: the row
/// the pointer was on and the entry that row shows. The app records one and
/// the presenter's tick takes it once
/// ([`App::take_pending_playlist_context`]); until S04 builds the menu from
/// it, taking the request is all that happens to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistContextRequest {
    /// The visible row the pointer was on (0-based, in the list).
    pub row: usize,
    /// The entry that row shows: the subject the menu would act on.
    pub entry: EntryId,
}

impl App {
    /// Takes the MISC popup request once, for the presenter's native-menu drain.
    pub fn take_playlist_misc_menu(&mut self) -> bool {
        std::mem::take(&mut self.pending_playlist_misc_menu)
    }

    /// Takes the pending right-click context-menu request once (S03 §5), for
    /// the presenter's tick.
    ///
    /// **Blocked on S04**: the playlist's context menu — its rows, wording and
    /// enabled states — is not implemented yet, so the request is taken and
    /// nothing is shown. S04 builds the menu from this value.
    pub fn take_pending_playlist_context(&mut self) -> Option<PlaylistContextRequest> {
        self.pending_playlist_context.take()
    }

    /// The playlist window's client size at rest (S06 scope; S01 §1).
    pub fn playlist_window_size() -> (u32, u32) {
        PLAYLIST_WINDOW_SIZE
    }

    /// Whether the playlist editor is drawn in its windowshade form.
    ///
    /// The editor's shade band is 14 px tall (S01 §1); the drawing itself is
    /// the skin's ([`rustamp_ui::render_playlist_window`]).
    pub fn playlist_shaded(&self) -> bool {
        self.playlist.windowshade
    }

    /// The client area the playlist editor's OS window must have right now.
    ///
    /// The unscaled size persisted for the playlist, regardless of shade.
    fn playlist_logical_size(&self) -> (u32, u32) {
        if let Some(layout) = self
            .config()
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Playlist)
            && let (Some(w), Some(h)) = (layout.width, layout.height)
        {
            (w.clamp(275, 4096), h.clamp(116, 4096))
        } else {
            window_size(WindowKind::Playlist, false, false)
        }
    }

    /// Apply the user's 2x policy without resizing an already large playlist.
    pub(crate) fn update_playlist_double_size(&mut self, enabled: bool) {
        if !enabled && self.playlist_double_size && self.playlist_resized_in_double_size {
            let (width, height) = self.playlist_logical_size();
            self.playlist_double_size = false;
            self.playlist_resized_in_double_size = false;
            self.set_playlist_size(width * 2, height * 2);
            return;
        }
        let (width, height) = self.playlist_logical_size();
        self.playlist_double_size = enabled && width < 550 && height < 232;
        self.playlist_resized_in_double_size = false;
    }

    /// Scaling used for the playlist's pixels and pointer coordinates.
    pub fn playlist_scale(&self) -> u32 {
        if self.playlist_double_size { 2 } else { 1 }
    }

    /// Current physical client size, including the optional 2x skin scale.
    pub fn playlist_frame_size(&self) -> (u32, u32) {
        let size = self.playlist_logical_size();
        let scale = self.playlist_scale();
        if self.playlist_shaded() {
            (size.0 * scale, 14 * scale)
        } else {
            (size.0 * scale, size.1 * scale)
        }
    }

    /// Update the playlist's saved and live dimensions during a resize.
    pub fn set_playlist_size(&mut self, width: u32, height: u32) {
        let scale = self.playlist_scale();
        let width = (width / scale).clamp(275, 4096 / scale);
        let height = (height / scale).clamp(116, 4096 / scale);
        if self.playlist_double_size && self.playlist_logical_size() != (width, height) {
            self.playlist_resized_in_double_size = true;
        }
        if let Some(layout) = self
            .config_mut()
            .ui
            .windows
            .iter_mut()
            .find(|w| w.id == WindowId::Playlist)
        {
            layout.width = Some(width);
            layout.height = Some(height);
        } else {
            let layout = rustamp_core::config::WindowLayout {
                id: WindowId::Playlist,
                x: 0,
                y: 0,
                visible: true,
                shaded: false,
                width: Some(width),
                height: Some(height),
            };
            self.config_mut().ui.windows.push(layout);
        }
        self.playlist.set_size(width, height);
    }

    /// The docked stack for the player's current state (S05 §3.3, §5).
    ///
    /// The main window's shade and double-size, the equalizer's shade and
    /// double-size and the editor's own shade are replayed through
    /// [`DockedStack`], so the three windows keep their edge connections
    /// (equalizer top = main bottom, playlist top = equalizer bottom) and a
    /// shaded editor is one row tall. The presenter keeps a *live* stack
    /// (dragging moves it); this is the state-derived one, the same geometry
    /// the fresh launch starts from.
    pub fn docked_stack(&self) -> DockedStack {
        let mut stack = DockedStack::default_launch();
        let state = self.state();
        stack.set_main_shaded(state.windowshade);
        stack.set_main_double_size(state.double_size);
        stack.set_equalizer_double_size(self.eq_state().double_size);
        stack.set_equalizer_shaded(self.eq_state().windowshade);
        stack.set_playlist_shaded(self.playlist_shaded());
        let (width, height) = self.playlist_frame_size();
        stack.playlist.width = width;
        stack.playlist.height = height;
        stack
    }

    /// Where the playlist window's client origin sits in the docked stack.
    ///
    /// Replays the windows' shade and double-size state through the S05
    /// docked geometry, so the playlist tracks the main window's bottom edge
    /// (through the equalizer's). At rest this is (26,261); with the main
    /// shaded it is (26,159) (S05 §3.3). The editor's own shade does not move
    /// it: it is the bottom window of the stack.
    pub fn playlist_window_origin(&self) -> (i32, i32) {
        let stack = self.docked_stack();
        (stack.playlist.x, stack.playlist.y)
    }

    /// The playlist behaviour model (the list view the window shows).
    pub fn playlist(&self) -> &rustamp_ui::PlaylistWindow {
        &self.playlist
    }

    /// The playlist editor's runtime font size, when the user chose one.
    ///
    /// `None` means the skin's size is in use (S17 §5). The choice is
    /// persisted with the other UI state, so it survives a restart.
    pub fn playlist_font_size(&self) -> Option<u32> {
        self.playlist.font_size().user_size()
    }

    /// Restores the user's chosen playlist font size (or clears it).
    ///
    /// This is the path a persisted choice comes back through; the skin's
    /// size stays in use when `None`.
    pub fn set_playlist_font_size(&mut self, size: Option<u32>) {
        self.playlist.set_font_size_override(size);
    }

    /// Pushes the skin's font size into the playlist model.
    ///
    /// The model owns the runtime font size (S03 §2 `40335` / `40336`); the
    /// skin's `Font=` line is its default (S17 §5). The app keeps the two in
    /// step: when the skin changes, the model's default follows it, and a
    /// size the user chose stays.
    pub(crate) fn sync_playlist_font_size(&mut self) {
        let size = rustamp_ui::skin_font_size(self.skin.skin());
        self.playlist.set_skin_font_size(size);
    }

    /// The playlist's title format, as the preferences page shows it.
    ///
    /// The user's setting, or the default: the format **off** with the
    /// engine's default template in the field. Without a setting the rows
    /// show their plain text, which is what the playlist shows today.
    pub fn playlist_title_format(&self) -> TitleFormatSetting {
        self.title_format.clone().unwrap_or_default()
    }

    /// Sets the playlist's title format.
    ///
    /// `enabled` is the switch, `format` the format string. The reason the
    /// format string cannot be used is returned: the setting is kept either
    /// way, so the preferences page shows the user's text and the user can
    /// fix it, but the rows keep their plain text while the format cannot be
    /// used.
    pub fn set_playlist_title_format(&mut self, enabled: bool, format: &str) -> Result<(), String> {
        let setting = TitleFormatSetting::new(enabled, format);
        let problem = setting.check().err();
        self.title_format = Some(setting);
        match problem {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
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
    /// Buttons fire on press; rows arm a headless drag (S06 §8 Q6 is open);
    /// the scrollbar pages the view or takes hold of its handle (S06 §3).
    ///
    /// The **modifiers** choose the list gesture (own choice; S06 §2 settles
    /// only the single click): no modifier selects the pressed row alone,
    /// `Shift` extends the selection from the anchor to the pressed row and
    /// `Ctrl` toggles exactly the pressed row. The model owns the rules and
    /// produces the [`Command::Select`] the app applies.
    pub fn playlist_mouse_down(
        &mut self,
        x: i32,
        y: i32,
        modifiers: Modifiers,
    ) -> Result<Vec<Command>> {
        let (width, height) = self.playlist_logical_size();
        let scale = self.playlist_scale() as i32;
        let (x, y) = (x / scale, y / scale);
        self.playlist.set_size(width, height);
        self.sync_playlist_entries();
        self.playlist.cancel_drag();
        if self.playlist_shaded() {
            // A shaded editor is a 14 px title strip (S01 §1): it has no list
            // and no bottom bar, so only its title-bar buttons answer.
            return match hit_test_title_bar(x - (width as i32 - 275), y) {
                Some(button) => self.playlist_title_button(button),
                None => Ok(Vec::new()),
            };
        }
        let bottom_y = y - (height as i32 - 116);
        let right_x = x - (width as i32 - 275);
        let button = hit_test_button(x, bottom_y)
            .filter(|b| *b != PlaylistButton::ListOpts)
            .or_else(|| {
                hit_test_button(right_x, bottom_y).filter(|b| *b == PlaylistButton::ListOpts)
            });
        if let Some(button) = button {
            return self.playlist_press(button);
        }
        if self.playlist.contains_list_point(x, y) {
            return match self.playlist.mouse_down(x, y, modifiers) {
                Some(command) => {
                    let commands = vec![command.clone()];
                    self.submit(command)?;
                    Ok(commands)
                }
                None => Ok(Vec::new()),
            };
        }
        // The scrollbar answers in three parts (S06 §3): the arrow pair pages
        // the view, the handle takes hold of it (a dragged handle moves the
        // view, not a selection) and the track pages it as well (own choice,
        // see `PlaylistWindow::press_scrollbar`). The view is the model's own
        // state, so a scrollbar press sends no command.
        if self.playlist.press_scrollbar(x, y) {
            return Ok(Vec::new());
        }
        if let Some(button) = hit_test_title_bar(right_x, y) {
            return self.playlist_title_button(button);
        }
        Ok(Vec::new())
    }

    /// A **right**-button press at playlist-window-relative `(x, y)`, S03 §5.
    ///
    /// The measured gesture is a right click on a playlist **entry**: it
    /// selects that entry and opens the playlist's context menu for it. The
    /// menu is **blocked on S04** — its rows, wording and enabled states are
    /// not measured in this build and are not invented here — so this half
    /// does exactly what S03 §5 settles for the click itself:
    ///
    /// * a press on an entry **selects** it through the selection machinery
    ///   the left click uses (`PlaylistWindow::mouse_down` with no modifier,
    ///   whose [`Command::Select`] the app submits like any other click), so
    ///   there is one selection path and no second one for the right button;
    /// * the row drag that press arms is cancelled again: the right button
    ///   moves no rows and its release must not turn into a `MoveEntries`;
    /// * the request for the menu is recorded on the app
    ///   ([`App::take_pending_playlist_context`]), where the presenter's tick
    ///   takes it once.
    ///
    /// A press that lands on **no entry** — the space below the last one, the
    /// bottom bar, the title bar, a shaded editor — changes nothing and asks
    /// for no menu: there is no entry for a menu to act on. The reference's
    /// behaviour there is unmeasured.
    pub fn playlist_right_mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let (width, height) = self.playlist_logical_size();
        let scale = self.playlist_scale() as i32;
        let (x, y) = (x / scale, y / scale);
        self.playlist.set_size(width, height);
        self.sync_playlist_entries();
        if self.playlist_shaded() {
            return Ok(Vec::new());
        }
        let Some(row) = self.playlist.row_at(x, y) else {
            return Ok(Vec::new());
        };
        // A row band below the last entry holds no entry: nothing to select
        // and no menu to ask for.
        let Some(entry) = self.playlist.visible_ids().get(row).copied() else {
            return Ok(Vec::new());
        };
        let commands = match self.playlist.mouse_down(x, y, Modifiers::NONE) {
            Some(command) => {
                let commands = vec![command.clone()];
                self.submit(command)?;
                commands
            }
            None => Vec::new(),
        };
        self.playlist.cancel_drag();
        self.pending_playlist_context = Some(PlaylistContextRequest { row, entry });
        Ok(commands)
    }

    /// A left-button release at playlist-window-relative `(x, y)`.
    ///
    /// A completed drag uses `MoveEntries`, sharing the sort/reorder application
    /// path. Plain releases and cancelled/unchanged drags emit nothing.
    ///
    /// The release also ends a held scrollbar handle: the view stays where
    /// the drag left it (S06 §3).
    pub fn playlist_mouse_up(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let scale = self.playlist_scale() as i32;
        let (x, y) = (x / scale, y / scale);
        self.sync_playlist_entries();
        self.playlist.release_scrollbar();
        if let Some(command) = self.playlist.mouse_up(x, y) {
            self.submit(command.clone())?;
            return Ok(vec![command]);
        }
        Ok(Vec::new())
    }

    /// Feeds a playlist-relative pointer position to the headless drag model.
    ///
    /// A held scrollbar handle takes the pointer first: it moves the view
    /// (S06 §3) and the row drag stays unarmed. Without a held handle the
    /// pointer feeds the row drag.
    pub fn playlist_mouse_move(&mut self, x: i32, y: i32) {
        let scale = self.playlist_scale() as i32;
        let (x, y) = (x / scale, y / scale);
        if self.playlist.drag_scrollbar(y) {
            return;
        }
        self.playlist.mouse_move(x, y);
    }

    /// Cancels a row drag on pointer leave or focus loss, without moving rows.
    ///
    /// A held scrollbar handle is released with it: the view stays where the
    /// drag left it.
    pub fn playlist_mouse_leave(&mut self) {
        self.playlist.cancel_drag();
        self.playlist.release_scrollbar();
    }

    /// A double-click at playlist-window-relative `(x, y)` (S03 §5, S06 §8 Q5).
    ///
    /// Double-clicking a visible row selects it and starts playback of that
    /// entry ([`Command::PlayEntry`]).
    pub fn playlist_double_click(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let scale = self.playlist_scale() as i32;
        let (x, y) = (x / scale, y / scale);
        self.sync_playlist_entries();
        self.playlist.cancel_drag();
        if self.playlist_shaded() {
            // A shaded editor is a 14 px title strip (S01 §1): it has no list.
            return Ok(Vec::new());
        }
        if let Some(row) = self.playlist.row_at(x, y)
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
    /// * the rows [`rustamp_ui::playlist::PLAYLIST_TABLE`] names — every row
    ///   of S03 §2 this build can act on, from the selection moves and jumps
    ///   (provisional) over Enter (`40184`, play the selection), `Delete`
    ///   (`1034`) and `Ctrl+Delete` (`1035`, crop), the four `Ctrl+Shift`
    ///   reorder rows, the two whole-list removal presets (`Alt+Delete`
    ///   "Remove all dead files", `40222`; `Ctrl+Shift+Delete` or `Ctrl+N`
    ///   "Clear the playlist", `40214`), the add rows (`L`, `Ctrl+L`,
    ///   `Shift+L`, `Insert`, `Ctrl+Insert`, `Shift+Insert`, `Num0`,
    ///   `Ctrl+Num0` — `1032`, `1036`, `1039`), the list rows (`Ctrl+A`
    ///   "Select all", `40205`; `Ctrl+I` "Invert selection", `40171`;
    ///   `Ctrl+S` "Save playlist", `40204`; `Ctrl+O` "Open playlist",
    ///   `40202`; `Ctrl+R` "Reverse the list", `40213`) and the shared rows
    ///   of S03 §1/§4;
    /// * the S03 §2 rows this build cannot act on yet
    ///   ([`rustamp_ui::playlist::UNIMPLEMENTED_PLAYLIST_ROWS`]): they are
    ///   bound to their command id, but no command implements them, so the
    ///   app records a note and does nothing;
    /// * every other key takes the shared shortcut path ([`App::key_down`]).
    ///
    /// S03 leaves how the tables combine provisional, so a key the playlist
    /// table does not bind keeps whatever binding the shared path has.
    pub fn playlist_key_down(
        &mut self,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<Option<ShortcutOutcome>> {
        use rustamp_ui::playlist::{playlist_key_action, unimplemented_playlist_row};

        self.sync_playlist_entries();

        if key == Key::Escape {
            // Escape ends a held gesture: the row drag and the scrollbar
            // handle (own choice, like the row drag below).
            let cancelled = self.playlist.cancel_drag();
            let released = self.playlist.release_scrollbar();
            if cancelled || released {
                return Ok(Some(ShortcutOutcome::Handled));
            }
        }
        // A keyboard action must not change selection/viewport underneath a
        // held row gesture. Like the drag itself this is an own-choice rule.
        self.playlist.cancel_drag();
        self.playlist.release_scrollbar();

        // Own choice, and not an S03 row: `Shift+Up` and `Shift+Down` extend
        // the selection from the anchor of the range gestures. S03 §2 binds
        // neither chord in the playlist table, and the shared tables bind
        // neither either, so the editor's own table has no row for them.
        if modifiers == Modifiers::SHIFT && matches!(key, Key::Up | Key::Down) {
            return match self.playlist.extend_selection(key == Key::Down) {
                Some(command) => {
                    self.submit(command.clone())?;
                    Ok(Some(ShortcutOutcome::Commands(vec![command])))
                }
                // Nothing to extend: the gesture changed nothing.
                None => Ok(Some(ShortcutOutcome::Handled)),
            };
        }

        // The playlist table (S03 §2) comes before the shared path: it is the
        // editor's own table and shadows the main and global rows where the
        // two disagree.
        if let Some(action) = playlist_key_action(key, modifiers) {
            return self.dispatch_playlist_action(action);
        }

        // A row S03 §2 binds but this build has no command for: note it and
        // do nothing, so the shared path cannot run a different action for
        // the same chord.
        if let Some(row) = unimplemented_playlist_row(key, modifiers) {
            self.note(format!(
                "playlist shortcut {:?} (command id {}) has no command yet",
                row.key, row.command_id
            ));
            return Ok(Some(ShortcutOutcome::Handled));
        }

        // Everything else takes the shared shortcut path.
        self.key_down(key, modifiers)
    }

    /// Runs one S03 §2 action.
    ///
    /// The shared rows (S03 §1/§4) go through the main window's machine, so
    /// the prediction and the command set are the same wherever the key was
    /// pressed ([`App::dispatch_shared_shortcut`]).
    pub(crate) fn dispatch_playlist_action(
        &mut self,
        action: PlaylistKeyAction,
    ) -> Result<Option<ShortcutOutcome>> {
        match action {
            // ----- the shared S03 rows (S03 §1/§4) -----
            PlaylistKeyAction::Shared(shortcut) => {
                let outcome = self.dispatch_shared_shortcut(shortcut)?;
                Ok(Some(outcome))
            }
            // ----- navigation (provisional, no measured effect) -----
            PlaylistKeyAction::MoveSelectionUp => {
                self.playlist.move_selection_up();
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::MoveSelectionDown => {
                self.playlist.move_selection_down();
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::JumpToFirst => {
                self.playlist.jump_to_first();
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::JumpToLast => {
                self.playlist.jump_to_last();
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::PageUp => {
                self.playlist
                    .scroll_page(rustamp_ui::playlist::ScrollDirection::Up);
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::PageDown => {
                self.playlist
                    .scroll_page(rustamp_ui::playlist::ScrollDirection::Down);
                Ok(Some(ShortcutOutcome::Handled))
            }
            // ----- the block moves (provisional, no measured effect) -----
            PlaylistKeyAction::MoveBlockUp => self.playlist_block_move(true),
            PlaylistKeyAction::MoveBlockDown => self.playlist_block_move(false),
            // ----- play the selection (menu-confirmed) -----
            PlaylistKeyAction::PlaySelected => match self.playlist.play_selected() {
                Some(command) => {
                    self.submit(command.clone())?;
                    Ok(Some(ShortcutOutcome::Commands(vec![command])))
                }
                None => Ok(Some(ShortcutOutcome::Handled)),
            },
            // ----- remove, crop, clear (menu-confirmed) -----
            PlaylistKeyAction::Delete => match self.playlist.press_rem() {
                Some(command) => {
                    self.submit(command.clone())?;
                    Ok(Some(ShortcutOutcome::Commands(vec![command])))
                }
                // S06 §5.3: Delete with nothing selected changes nothing.
                None => Ok(Some(ShortcutOutcome::Handled)),
            },
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
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // Remove all dead files (menu-confirmed, Alt+Delete, 40222).
            // The app resolves which entries are dead, so the command
            // carries no payload (RFC 0018).
            PlaylistKeyAction::RemoveDead => {
                let command = Command::RemoveDeadFiles;
                self.submit(command.clone())?;
                self.sync_playlist_entries();
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // Clear the playlist (menu-confirmed, Ctrl+Shift+Delete and
            // Ctrl+N, both id 40214, S03 §2).
            PlaylistKeyAction::ClearPlaylist => {
                let command = Command::Clear;
                self.submit(command.clone())?;
                self.sync_playlist_entries();
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // ----- the reorder rows (menu-confirmed) -----
            // The app sorts or randomizes its entry-id table and the model
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
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            PlaylistKeyAction::Randomize => {
                let command = Command::Randomize;
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            PlaylistKeyAction::Reverse => {
                let command = Command::Reverse;
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // ----- the selection rows (menu-confirmed) -----
            PlaylistKeyAction::SelectAll => {
                let command = self.playlist.select_all();
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            PlaylistKeyAction::InvertSelection => {
                let command = self.playlist.invert_selection();
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // ----- the add rows (menu-confirmed) -----
            // The three add commands are the playlist's own (`1032`, `1036`,
            // `1039`), so they open the app's media dialog with the
            // matching face. Their `play` flag is unmeasured (RFC 0005 O5),
            // so it takes the ADD button's `false`.
            PlaylistKeyAction::AddFiles => self.open_media(OpenMediaKind::Files),
            PlaylistKeyAction::AddLocation => self.open_media(OpenMediaKind::Location),
            PlaylistKeyAction::AddDirectory => self.open_media(OpenMediaKind::Directory),
            // ----- the remaining menu-confirmed rows -----
            PlaylistKeyAction::OpenPlaylist => {
                let command = Command::OpenPlaylist;
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            PlaylistKeyAction::SavePlaylist => {
                // The presenter opens the native save dialog and sends
                // `Command::SavePlaylist` with the chosen path.
                self.request_save_playlist = true;
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::FileInfo => {
                // S03 §6 Q6 leaves open whether the playlist's "File info"
                // (`40208`) and the main window's "View file info" (`40188`)
                // are the same dialog; the app has one file-info box, so the
                // playlist's row opens it.
                let command = Command::OpenFileInfo;
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            PlaylistKeyAction::BookmarkSelection => {
                // S03 §2's row bookmarks the *selected* item(s), where the
                // main window's bookmarks the current track.
                self.bookmark_selection();
                Ok(Some(ShortcutOutcome::Commands(Vec::new())))
            }
            // ----- the HTML playlist (S03 §2 `40292`) -----
            // The document goes where the user says, so the app records the
            // request and the presenter asks for a file: writing a file is
            // not a player action and needs no core command (RFC 0005 §9).
            PlaylistKeyAction::GenerateHtmlPlaylist => {
                self.request_html_playlist = true;
                Ok(Some(ShortcutOutcome::Handled))
            }
            // ----- the extended info of the selection (S03 §2 `40293`) -----
            // The tags of the selected entries are read and shown in the
            // extended-info box. No core command opens that box (reading a
            // file's tags is not a player action, RFC 0005 §9), so the app
            // opens it itself; the action is handled either way.
            PlaylistKeyAction::ReadExtendedInfo => {
                self.show_extended_info();
                Ok(Some(ShortcutOutcome::Handled))
            }
            // ----- the edit-entry dialog (S03 §2 `40255`) -----
            // The row is bound: the dialog's model (the fields, the
            // validation and the save request) lives in `rustamp-ui`'s
            // `dialogs` module, and the app half — reading the tags of the
            // selected entry, opening the dialog and writing the changed
            // fields back with `rustamp-meta` — is in `crate::dialogs`. The
            // action is handled either way; it does not fall through to the
            // main table's `40186` "Toggle easy move".
            PlaylistKeyAction::EditEntry => {
                self.show_edit_entry();
                Ok(Some(ShortcutOutcome::Handled))
            }
            // ----- the provisional close row -----
            PlaylistKeyAction::CloseEditor => {
                let commands = self.close_playlist_window()?;
                if commands.is_empty() {
                    Ok(Some(ShortcutOutcome::Handled))
                } else {
                    Ok(Some(ShortcutOutcome::Commands(commands)))
                }
            }
            // ----- the editor's own windowshade -----
            PlaylistKeyAction::ToggleEditorShade => {
                let command = Command::ToggleWindowShade(WindowId::Playlist);
                self.submit(command.clone())?;
                Ok(Some(ShortcutOutcome::Commands(vec![command])))
            }
            // ----- the runtime font size (S03 §2 `40335` / `40336`) -----
            // A view setting of this window: it changes the rows the editor
            // draws and nothing else, so it needs no core command
            // (RFC 0005 §9/§11.6).
            PlaylistKeyAction::IncreaseFontSize => {
                self.playlist.increase_font_size();
                Ok(Some(ShortcutOutcome::Handled))
            }
            PlaylistKeyAction::DecreaseFontSize => {
                self.playlist.decrease_font_size();
                Ok(Some(ShortcutOutcome::Handled))
            }
        }
    }

    /// Moves the selected block one row up or down (S03 §2 `40215`/`40216`).
    ///
    /// The model owns the selection and the order, so it computes the target
    /// and emits the ordinary [`Command::MoveEntries`]; the app applies it
    /// and the model re-syncs. Nothing selected, or a block that already
    /// starts or ends the list, changes nothing.
    fn playlist_block_move(&mut self, up: bool) -> Result<Option<ShortcutOutcome>> {
        let command = if up {
            self.playlist.move_block_up()
        } else {
            self.playlist.move_block_down()
        };
        let Some(command) = command else {
            return Ok(Some(ShortcutOutcome::Handled));
        };
        self.submit(command.clone())?;
        self.sync_playlist_entries();
        Ok(Some(ShortcutOutcome::Commands(vec![command])))
    }

    /// Opens the app's media dialog for the playlist's add rows.
    ///
    /// `play` is `false`: the add commands' `play` flag is unmeasured
    /// (RFC 0005 O5) and the ADD button takes `false` as well.
    fn open_media(&mut self, kind: OpenMediaKind) -> Result<Option<ShortcutOutcome>> {
        let command = Command::OpenMedia { kind, play: false };
        self.submit(command.clone())?;
        Ok(Some(ShortcutOutcome::Commands(vec![command])))
    }

    /// Renders one frame of the playlist window.
    pub fn render_playlist(&mut self) -> Image {
        let (width, height) = self.playlist_logical_size();
        self.playlist.set_size(width, height);
        // The skin's font size is the playlist's size until the user changes
        // it (S17 §5, S03 §2 `40335` / `40336`), so the model is told the
        // skin's size before the frame is drawn.
        self.sync_playlist_font_size();
        // Playback, not selection, owns Current (S17 section 5). Read the
        // engine on every paint, including natural advancement and no track.
        let index = self.engine().current_index();
        let current = self.entry_id_for(index);
        self.playlist.set_current(current);
        let ids: Vec<EntryId> = self.playlist.visible_ids();
        // The tags of the rows are read off the UI thread (see
        // [`crate::playlist_tags`]): the ones that arrived are taken, and the
        // rows that have none yet are asked for.
        self.playlist_tags.poll();
        self.request_row_tags(&ids);
        self.sync_playlist_status();
        // The format is read once for the frame: every row shows the same
        // one.
        let format = self.playlist_title_format().engine_format();
        let labels: Vec<String> = ids
            .iter()
            .map(|id| self.playlist_label(*id, &format))
            .collect();
        let focused = self.focused_for_playlist();
        let image = render_playlist_window(self.skin.skin(), &self.playlist, &labels, focused);
        if self.playlist_scale() == 2 {
            image.scaled(2)
        } else {
            image
        }
    }

    /// Asks for the tags of the rows that have none.
    ///
    /// The tags are read off the UI thread, so a row shows its plain text
    /// until its tags arrive. Only the rows the playlist shows are asked
    /// for: a player that never shows a row never reads a file.
    fn request_row_tags(&mut self, ids: &[EntryId]) {
        for id in ids {
            if self.playlist_tags.metadata(*id).is_some() {
                continue;
            }
            let index = self.entry_ids.iter().position(|entry| entry == id);
            let Some(index) = index else {
                continue;
            };
            let path = match self.engine().queue().get(index) {
                Some(TrackSource::Path(path)) => Some(path.clone()),
                _ => None,
            };
            let Some(path) = path else {
                continue;
            };
            self.playlist_tags.request(*id, &path);
        }
    }

    /// The row text for one entry: its position in the list (S06 §5: the rows
    /// renumber after a removal) and its name, which is the tags once they
    /// arrived and the file name until then. It falls back to the entry id when
    /// it has no queue row (for example after a REM that the engine queue does
    /// not track yet).
    ///
    /// With the title format turned on (spec S14), the row shows the format
    /// the user set instead, rendered with the fields the row carries.
    fn playlist_label(&self, id: EntryId, format: &TitleFormat) -> String {
        let index = self.entry_ids.iter().position(|entry| *entry == id);
        let source = index.and_then(|index| self.engine().queue().get(index));
        let plain = match source {
            Some(_) => {
                let name = index
                    .and_then(|index| self.entry_name(index))
                    .unwrap_or_default();
                // A row without a name stays an empty row.
                if name.is_empty() {
                    String::new()
                } else {
                    format!("{}. {name}", index.map_or(0, |index| index + 1))
                }
            }
            None => format!("{id:?}"),
        };
        // The renderer right-aligns what follows the tab.
        let length = self
            .playlist_tags
            .metadata(id)
            .and_then(|tags| tags.duration_ms)
            .filter(|_| !plain.is_empty())
            .map_or_else(String::new, |ms| {
                let seconds = ms / 1000;
                format!("\t{}:{:02}", seconds / 60, seconds % 60)
            });
        if !format.is_enabled() {
            return format!("{plain}{length}");
        }
        let Some(track) = source else {
            // A row the queue does not know has no fields: it keeps the
            // plain text, which names the row.
            return plain;
        };
        let fields = crate::title_format::row_fields(track, self.playlist_tags.metadata(id));
        format!("{}{length}", format.row_text(&plain, &fields))
    }

    /// Hands the bottom bar its two fields: the length of the selection and of
    /// the list, and the time of the open track. A length the player does not
    /// know yet counts as nothing and shows as a `+` behind the total (own
    /// choice).
    fn sync_playlist_status(&mut self) {
        let all = self.entry_ids.clone();
        self.request_row_tags(&all);
        let length = |id: &EntryId| {
            self.playlist_tags
                .metadata(*id)
                .and_then(|tags| tags.duration_ms)
        };
        let seconds = |ms: u64| ms / 1000;
        let total: u64 = all.iter().filter_map(length).map(seconds).sum();
        let selected: u64 = self
            .playlist
            .selected_ids()
            .iter()
            .filter_map(length)
            .map(seconds)
            .sum();
        let unknown = if all.iter().all(|id| length(id).is_some()) {
            ""
        } else {
            "+"
        };
        let clock = |secs: u64| format!("{}:{:02}", secs / 60, secs % 60);
        self.playlist.info_text = format!("{}/{}{unknown}", clock(selected), clock(total));
        let state = self.state();
        let digits = state.time_cells();
        self.playlist.time_digits = (state.time_visible
            && !state.shows_time_placeholder()
            && digits.iter().all(Option::is_some))
        .then(|| digits.map(|digit| digit.unwrap_or(0)));
    }

    /// Mouse-wheel notches over the playlist editor: three rows a notch (own
    /// choice: S06 §3 only measures the arrows, which page).
    pub fn playlist_wheel(&mut self, notches: i32) {
        let (width, height) = self.playlist_logical_size();
        self.playlist.set_size(width, height);
        self.sync_playlist_entries();
        let target = self
            .playlist
            .first_visible()
            .saturating_add_signed(-(notches as isize) * 3);
        self.playlist.scroll_to(target);
    }

    /// Which focus variant the playlist frame renders with.
    ///
    /// The playlist's OWN focus flag. Sharing the main window's flag swapped
    /// both states in the capture set (measured 2026-10-21): `playlist-active`
    /// showed the unfocused band and `playlist-inactive` the focused one, each
    /// by exactly the 1 388 pixels S06 section 5.1 names for the band change.
    fn focused_for_playlist(&self) -> bool {
        self.playlist_focused
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
                self.pending_playlist_misc_menu = true;
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
    /// PL button reopens it.
    ///
    /// Shade toggles the editor's windowshade: the editor collapses to its
    /// 14 px title strip and expands again (S01 §1; the band itself is the
    /// skin's). The state travels as
    /// [`Command::ToggleWindowShade`](rustamp_core::command::Command::ToggleWindowShade)
    /// for the playlist, so the app applies it in one place and the presenter
    /// reads the new size from [`App::playlist_frame_size`].
    /// A (6,3) click never reaches here ([`hit_test_title_bar`] returns
    /// `None`, S06 §1).
    fn playlist_title_button(&mut self, button: PlaylistTitleButton) -> Result<Vec<Command>> {
        match button {
            PlaylistTitleButton::Close => self.close_playlist_window(),
            PlaylistTitleButton::Shade => {
                let command = Command::ToggleWindowShade(WindowId::Playlist);
                self.submit(command.clone())?;
                Ok(vec![command])
            }
        }
    }

    /// Hides the playlist editor when it is open (its title-bar Close and its
    /// OS close box). Returns the command it sent, if any.
    pub fn close_playlist_window(&mut self) -> Result<Vec<Command>> {
        self.playlist.cancel_drag();
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
    use rustamp_core::event::Event;
    use rustamp_core::meta::TrackMetadata;
    use rustamp_core::playlist::{EntryId, MediaLocation};
    use rustamp_core::state::WindowId;
    use rustamp_platform::PrefsAction;

    use super::*;
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    /// The playlist's title band has a focused and an unfocused variant (S01
    /// §4.1), and the captures proved the two windows disagree: sharing the
    /// main window's flag swapped both playlist states by the 1 388 pixels S06
    /// §5.1 names. This pins the separation; before the fix, `set_focused(true)`
    /// alone flipped the playlist.
    #[test]
    fn the_playlist_uses_its_own_focus_and_not_the_main_windows() {
        let mut app = test_app();
        assert!(!app.focused_for_playlist(), "the playlist opens unfocused");
        app.set_focused(true);
        assert!(
            !app.focused_for_playlist(),
            "the main window's focus is not the playlist's"
        );
        app.set_playlist_focused(true);
        assert!(app.focused_for_playlist(), "the playlist's own flag counts");
    }

    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
            seconds: None,
        };
        App::new(options).expect("the test app builds headless")
    }

    #[test]
    fn resized_playlist_frame_and_controls_follow_saved_size() {
        let mut app = test_app();
        app.set_playlist_size(550, 348);
        let frame = app.render_playlist();
        assert_eq!((frame.width(), frame.height()), (550, 348));
        let commands = app.playlist_mouse_down(24, 324, Modifiers::NONE).unwrap();
        assert!(!commands.is_empty(), "ADD moves with the bottom edge");
        let commands = app.playlist_mouse_down(517, 324, Modifiers::NONE).unwrap();
        assert!(!commands.is_empty(), "LIST OPTS moves with the right edge");
        app.playlist_mouse_down(529, 3, Modifiers::NONE).unwrap();
        assert_eq!(app.playlist_frame_size(), (550, 14));
        assert_eq!(app.render_playlist().width(), 550);
        app.playlist_mouse_down(529, 3, Modifiers::NONE).unwrap();
        assert_eq!(app.playlist_frame_size(), (550, 348));
        app.set_playlist_size(275, 116);
        let frame = app.render_playlist();
        assert_eq!((frame.width(), frame.height()), (275, 116));
    }

    #[test]
    fn double_size_scales_a_small_playlist_and_restores_it() {
        let mut app = test_app();
        let original = app.render_playlist();
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.playlist_frame_size(), (550, 232));
        assert_eq!(app.render_playlist(), original.scaled(2));
        let commands = app.playlist_mouse_down(48, 184, Modifiers::NONE).unwrap();
        assert!(!commands.is_empty(), "the scaled ADD button is clickable");
        app.playlist_mouse_down(508, 6, Modifiers::NONE).unwrap();
        assert_eq!(app.playlist_frame_size(), (550, 28));
        app.playlist_mouse_down(508, 6, Modifiers::NONE).unwrap();
        assert_eq!(app.playlist_frame_size(), (550, 232));
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.playlist_frame_size(), (275, 116));
    }

    #[test]
    fn double_size_leaves_a_large_playlist_untouched() {
        let mut app = test_app();
        app.set_playlist_size(900, 584);
        let before = app.render_playlist();
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.playlist_frame_size(), (900, 584));
        assert_eq!(app.render_playlist(), before);
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.playlist_frame_size(), (900, 584));
    }

    #[test]
    fn manually_resizing_a_double_sized_playlist_keeps_the_new_size_on_exit() {
        let mut app = test_app();
        app.submit(Command::ToggleDoubleSize).unwrap();
        app.set_playlist_size(800, 400);
        assert_eq!(app.playlist_frame_size(), (800, 400));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        app.save_config_to(&path).unwrap();
        let saved = crate::config::load(Some(&path)).unwrap();
        let layout = saved
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Playlist)
            .unwrap();
        assert_eq!((layout.width, layout.height), (Some(800), Some(400)));
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.playlist_frame_size(), (800, 400));
        assert_eq!(app.playlist_scale(), 1);
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
    fn dragging_rows_reorders_queue_and_ids_at_both_ends_and_row_halves() {
        for (count, press_y, drop_y, expected) in [
            (4, 42, 74, vec![0, 2, 3, 1]), // One row to the bottom.
            (4, 68, 23, vec![3, 0, 1, 2]), // Last row to the top.
            (3, 28, 85, vec![1, 2, 0]),    // Blank space after a short list.
            (4, 28, 50, vec![1, 0, 2, 3]), // Before row 2.
            (4, 28, 60, vec![1, 2, 0, 3]), // After row 2.
        ] {
            let mut app = test_app();
            add_entries(&mut app, count);
            app.playlist_mouse_down(60, press_y, Modifiers::NONE)
                .unwrap();
            app.playlist_mouse_move(60, drop_y);
            let commands = app.playlist_mouse_up(60, drop_y).unwrap();
            assert!(matches!(commands.as_slice(), [Command::MoveEntries { .. }]));
            assert_eq!(
                app.entry_ids,
                expected.iter().copied().map(EntryId).collect::<Vec<_>>()
            );
            let expected_labels: Vec<_> =
                expected.iter().map(|id| format!("track{id}.mp3")).collect();
            let labels: Vec<_> = app
                .engine()
                .queue()
                .iter()
                .map(|source| source.display())
                .collect();
            assert_eq!(labels, expected_labels, "queue and ids must move together");
            assert_eq!(app.playlist().visible_ids(), app.entry_ids);
            assert!(app.drain_notes().is_empty());
        }
    }

    #[test]
    fn dragging_a_multi_selection_preserves_order_selection_and_current_identity() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.engine_mut().set_current_index(Some(2));
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(2), EntryId(0)])))
            .unwrap();
        assert!(
            app.playlist_mouse_down(60, 54, Modifiers::NONE)
                .unwrap()
                .is_empty()
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(2)]);
        app.playlist_mouse_move(60, 74);
        assert_eq!(
            app.playlist_mouse_up(60, 74).unwrap(),
            vec![Command::MoveEntries {
                ids: vec![EntryId(0), EntryId(2)],
                to: 2,
            }]
        );
        assert_eq!(
            app.entry_ids,
            vec![EntryId(1), EntryId(3), EntryId(0), EntryId(2)]
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(2)]);
        assert_eq!(app.engine().current_index(), Some(3));
        assert_current_frame(&mut app, Some(EntryId(2)));
    }

    #[test]
    fn same_position_click_escape_and_leave_never_reorder() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1), EntryId(2)])))
            .unwrap();
        let original_queue: Vec<_> = app
            .engine()
            .queue()
            .iter()
            .map(|source| source.display())
            .collect();
        for cancellation in 0..4 {
            app.playlist_mouse_down(60, 42, Modifiers::NONE).unwrap();
            let drop_y = if cancellation < 2 { 42 } else { 74 };
            if cancellation > 0 {
                app.playlist_mouse_move(64, drop_y);
            }
            match cancellation {
                2 => assert_eq!(
                    app.playlist_key_down(Key::Escape, Modifiers::NONE).unwrap(),
                    Some(ShortcutOutcome::Handled)
                ),
                3 => app.playlist_mouse_leave(),
                _ => {}
            }
            assert!(app.playlist_mouse_up(60, drop_y).unwrap().is_empty());
            assert_eq!(
                app.entry_ids,
                vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)]
            );
            assert_eq!(app.playlist().selected_ids(), vec![EntryId(1), EntryId(2)]);
            assert_eq!(app.playlist().drag_insertion(), None);
            assert_eq!(
                app.engine()
                    .queue()
                    .iter()
                    .map(|source| source.display())
                    .collect::<Vec<_>>(),
                original_queue
            );
        }
    }

    #[test]
    fn move_command_ignores_unknown_and_duplicate_ids_and_clamps_target() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.submit(Command::MoveEntries {
            ids: vec![EntryId(2), EntryId(99), EntryId(0), EntryId(2)],
            to: u32::MAX,
        })
        .unwrap();
        assert_eq!(
            app.entry_ids,
            vec![EntryId(1), EntryId(3), EntryId(0), EntryId(2)]
        );
        for ids in [vec![], vec![EntryId(99)]] {
            let before = app.entry_ids.clone();
            app.submit(Command::MoveEntries { ids, to: 0 }).unwrap();
            assert_eq!(app.entry_ids, before);
        }
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
            let rect = rustamp_ui::playlist_window::row_rect(row, app.playlist().row_pitch())
                .expect("the visible row");
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
                    // GDI blends the edges, so a pixel counts for what it is
                    // closest to.
                    ink_count += usize::from(
                        (0..3)
                            .map(|c| (i32::from(pixel[c]) - i32::from(ink[c])).pow(2))
                            .sum::<i32>()
                            < (0..3)
                                .map(|c| (i32::from(pixel[c]) - i32::from(bg[c])).pow(2))
                                .sum::<i32>(),
                    );
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
        // The first entry is current as soon as there is one, without playback
        // (Winamp 2.95 shows it in the marquee). There is no decoded metadata;
        // labels still come from each queue source.
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(
            app.engine().transport(),
            rustamp_core::state::TransportState::Stopped
        );
        assert_current_frame(&mut app, Some(EntryId(0)));
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
        let rect = rustamp_ui::playlist_window::row_rect(0, app.playlist().row_pitch())
            .expect("the first visible row");
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
        // New entries: the first is current, and it is a new id.
        assert_eq!(app.engine().current_index(), Some(0));
        let first = app.playlist().visible_ids()[0];
        assert_current_frame(&mut app, Some(first));
    }

    #[test]
    fn replacing_the_playlist_without_playing_makes_the_new_first_entry_current() {
        let mut app = test_app();
        add_entries(&mut app, 2);
        app.engine_mut().set_current_index(Some(1));
        assert_current_frame(&mut app, Some(EntryId(1)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replacement.m3u");
        std::fs::write(&path, "#EXTM3U\nreplacement0.mp3\nreplacement1.mp3\n").unwrap();
        app.load_playlist_file(&path, false).unwrap();
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(2), EntryId(3)]);
        assert_current_frame(&mut app, Some(EntryId(2)));
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
        let commands = app
            .playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("click applies");
        assert_eq!(
            commands,
            vec![Command::Select(SelectOp::Set(vec![EntryId(0)]))]
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
    }

    #[test]
    fn a_right_click_selects_the_row_and_asks_for_its_menu() {
        // S03 §5 measures a right click on a playlist entry: it opens the
        // playlist's context menu for that entry. The menu is S04's (blocked
        // on S04), so the click's own part is what is pinned here — one
        // `Command::Select` from the ordinary selection path, the row
        // selected, the request recorded exactly once, and no row drag left
        // armed for the release.
        let mut app = test_app();
        add_entries(&mut app, 6);
        let commands = app
            .playlist_right_mouse_down(60, 54)
            .expect("the right press applies");
        assert_eq!(
            commands,
            vec![Command::Select(SelectOp::Set(vec![EntryId(2)]))]
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(2)]);
        assert_eq!(
            app.take_pending_playlist_context(),
            Some(PlaylistContextRequest {
                row: 2,
                entry: EntryId(2),
            })
        );
        // Taken once: the next tick has nothing to take.
        assert_eq!(app.take_pending_playlist_context(), None);
        // The right press armed no drag: a release over another row would
        // otherwise move the selection there.
        assert!(app.playlist_mouse_up(60, 74).unwrap().is_empty());
        assert_eq!(app.entry_ids, (0..6).map(EntryId).collect::<Vec<_>>());
    }

    #[test]
    fn a_right_click_on_an_already_selected_row_keeps_the_selection() {
        // The menu acts on the **selection**, and the selection rule is the
        // one the left click already has: a press on a row that is part of the
        // selection changes nothing (a plain click on a selected row behaves
        // the same way), so the right click adds no command and only records
        // the request.
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("the first click selects row 0");
        app.playlist_mouse_down(60, 54, Modifiers::CTRL)
            .expect("the ctrl click adds row 2");
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(2)]);
        let commands = app
            .playlist_right_mouse_down(60, 54)
            .expect("the right press applies");
        assert!(commands.is_empty(), "{commands:?}");
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(2)]);
        assert_eq!(
            app.take_pending_playlist_context(),
            Some(PlaylistContextRequest {
                row: 2,
                entry: EntryId(2),
            })
        );
    }

    #[test]
    fn a_right_click_on_no_entry_selects_nothing_and_asks_nothing() {
        // The empty space below the last entry holds no entry for a menu to
        // act on: no selection change, no command and no request. The same
        // holds for the title bar, the bottom bar and a shaded editor.
        let mut app = test_app();
        add_entries(&mut app, 2);
        // The row bands below the two entries, the title strip and the bottom
        // bar: none of them names an entry.
        for (x, y) in [(60, 92), (254, 3), (24, 92)] {
            assert!(
                app.playlist_right_mouse_down(x, y).unwrap().is_empty(),
                "({x}, {y})"
            );
            assert_eq!(app.take_pending_playlist_context(), None, "({x}, {y})");
        }
        assert!(app.playlist().selected_ids().is_empty());
        // A shaded editor is one title strip and has no list at all.
        app.submit(Command::ToggleWindowShade(WindowId::Playlist))
            .expect("the editor shades");
        assert!(app.playlist_shaded());
        assert!(app.playlist_right_mouse_down(60, 3).unwrap().is_empty());
        assert_eq!(app.take_pending_playlist_context(), None);
    }

    /// The selection gestures reach the model through the modifiers, and the
    /// commands they produce keep the app's selection in step (own choice;
    /// S06 §2 settles only the single click).
    #[test]
    fn the_shift_click_extends_the_selection_and_the_ctrl_click_toggles_it() {
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("click applies");
        // Shift+click on row 2: the range from the anchor (row 0) to it.
        let commands = app
            .playlist_mouse_down(60, 54, Modifiers::SHIFT)
            .expect("shift click applies");
        assert_eq!(
            commands,
            vec![Command::Select(SelectOp::Set(vec![
                EntryId(0),
                EntryId(1),
                EntryId(2)
            ]))]
        );
        assert_eq!(
            app.playlist().selected_ids(),
            vec![EntryId(0), EntryId(1), EntryId(2)]
        );
        // Ctrl+click on row 3: exactly that row is toggled in.
        let commands = app
            .playlist_mouse_down(60, 68, Modifiers::CTRL)
            .expect("ctrl click applies");
        assert_eq!(
            commands,
            vec![Command::Select(SelectOp::Set(vec![
                EntryId(0),
                EntryId(1),
                EntryId(2),
                EntryId(3)
            ]))]
        );
        assert_eq!(
            app.playlist().selected_ids(),
            vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)]
        );
    }

    /// `Shift+Up`/`Shift+Down` extend the selection from the anchor (own
    /// choice: S03 §2 binds neither chord).
    #[test]
    fn the_shift_arrows_extend_the_selection_from_the_anchor() {
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("click applies");
        let outcome = app
            .playlist_key_down(Key::Down, Modifiers::SHIFT)
            .expect("the key applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Select(
                SelectOp::Set(vec![EntryId(0), EntryId(1)])
            )]))
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(1)]);
        // One row more, then one row back.
        app.playlist_key_down(Key::Down, Modifiers::SHIFT)
            .expect("the key applies");
        assert_eq!(
            app.playlist().selected_ids(),
            vec![EntryId(0), EntryId(1), EntryId(2)]
        );
        app.playlist_key_down(Key::Up, Modifiers::SHIFT)
            .expect("the key applies");
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0), EntryId(1)]);
    }

    #[test]
    fn sel_selects_all_visible_rows() {
        // S06 §2: SEL selects all visible rows.
        let mut app = test_app();
        add_entries(&mut app, 60);
        let commands = app
            .playlist_mouse_down(74, 92, Modifiers::NONE)
            .expect("SEL applies");
        let all = vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)];
        assert_eq!(commands, vec![Command::Select(SelectOp::Set(all.clone()))]);
        assert_eq!(app.playlist().selected_ids(), all);
    }

    #[test]
    fn rem_removes_the_staged_selection() {
        // S06 §5: REM removes the selected entry with no menu choice.
        let mut app = test_app();
        add_entries(&mut app, 60);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("stage a selection");
        let commands = app
            .playlist_mouse_down(46, 92, Modifiers::NONE)
            .expect("REM applies");
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
        let commands = app
            .playlist_mouse_down(46, 92, Modifiers::NONE)
            .expect("REM applies");
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
        let commands = app
            .playlist_mouse_down(24, 92, Modifiers::NONE)
            .expect("ADD applies");
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
        let commands = app
            .playlist_mouse_down(242, 92, Modifiers::NONE)
            .expect("LIST OPTS applies");
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
        app.playlist_mouse_down(24, 92, Modifiers::NONE)
            .expect("ADD applies");
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
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
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("stage a selection");
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

    /// S03 §2's two font-size rows are bound and act on the editor alone.
    ///
    /// `Ctrl+Num+` (`40335`) makes the rows taller, `Ctrl+Num-` (`40336`)
    /// shorter; the row height follows the font, so the page holds more rows
    /// with a smaller font. Neither row sends a command: the effect is
    /// confined to what the editor draws (RFC 0005 §9/§11.6).
    #[test]
    fn ctrl_num_add_and_ctrl_num_subtract_walk_the_playlist_font_size() {
        let mut app = test_app();
        add_entries(&mut app, 8);
        // The skin's size is in use until the user changes it (S17 §5).
        assert_eq!(app.playlist().font_size().size(), 13);
        assert!(!app.playlist().font_size().is_overridden());
        assert_eq!(app.playlist().visible_rows(), 4);

        let before = app.render_playlist();
        assert_eq!(
            app.playlist_key_down(Key::NumAdd, Modifiers::CTRL)
                .expect("the key applies"),
            Some(ShortcutOutcome::Handled),
            "a view setting sends no command"
        );
        assert_eq!(app.playlist().font_size().size(), 14);
        assert!(app.playlist().font_size().is_overridden());
        assert_eq!(app.playlist().row_pitch(), 14);
        assert_eq!(
            app.playlist().visible_rows(),
            3,
            "52 px hold three 14 px rows"
        );
        let bigger = app.render_playlist();
        assert_ne!(before, bigger, "the rows must follow the font");

        assert_eq!(
            app.playlist_key_down(Key::NumSubtract, Modifiers::CTRL)
                .expect("the key applies"),
            Some(ShortcutOutcome::Handled)
        );
        assert_eq!(app.playlist().font_size().size(), 13);
        assert_eq!(app.render_playlist(), before, "the size came back");

        // The size is clamped at both ends of the range.
        for _ in 0..100 {
            app.playlist_key_down(Key::NumAdd, Modifiers::CTRL)
                .expect("the key applies");
        }
        assert_eq!(app.playlist().font_size().size(), rustamp_ui::MAX_FONT_SIZE);
        for _ in 0..100 {
            app.playlist_key_down(Key::NumSubtract, Modifiers::CTRL)
                .expect("the key applies");
        }
        assert_eq!(app.playlist().font_size().size(), rustamp_ui::MIN_FONT_SIZE);
        assert!(app.drain_notes().is_empty());
    }

    /// The classic invariant at every size: a row holds the skin's ink and
    /// its background, never a blended shade (no antialiasing).
    #[test]
    fn the_playlist_rows_show_ink_on_their_background_at_every_size() {
        let mut app = test_app();
        add_entries(&mut app, 8);
        let style = app.skin.skin().pledit();
        let ink = [style.normal[0], style.normal[1], style.normal[2], 255];
        let bg = [
            style.normal_bg[0],
            style.normal_bg[1],
            style.normal_bg[2],
            255,
        ];
        for size in rustamp_ui::MIN_FONT_SIZE..=rustamp_ui::MAX_FONT_SIZE {
            app.set_playlist_font_size(Some(size));
            let frame = app.render_playlist();
            let pitch = app.playlist().row_pitch();
            for row in 0..app.playlist().visible_rows() {
                let rect =
                    rustamp_ui::playlist_window::row_rect(row, pitch).expect("the visible row");
                let mut ink_count = 0;
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        let pixel = frame.pixel(x, y).expect("a pixel");
                        ink_count += usize::from(
                            (0..3)
                                .map(|c| (i32::from(pixel[c]) - i32::from(ink[c])).pow(2))
                                .sum::<i32>()
                                < (0..3)
                                    .map(|c| (i32::from(pixel[c]) - i32::from(bg[c])).pow(2))
                                    .sum::<i32>(),
                        );
                    }
                }
                assert!(ink_count > 0, "size {size}, row {row}: no ink");
            }
        }
    }

    /// The skin's `Font=` size is the playlist's size; the user changes it
    /// from there (S17 §5, S03 §2 `40335` / `40336`).
    #[test]
    fn the_skins_font_size_is_the_playlist_size_until_the_user_changes_it() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        // A skin folder with its own `pledit.txt`: the loader reads the font
        // from it (the sheets fall back to the built-in default skin).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pledit.txt"),
            b"[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000C6\nFont=Courier New,10\n",
        )
        .unwrap();
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .expect("the skin loads");
        assert_eq!(app.playlist().font_size().size(), 10, "the skin's size");
        assert!(!app.playlist().font_size().is_overridden());
        assert_eq!(app.playlist().row_pitch(), 10);
        assert_eq!(app.playlist().visible_rows(), 5);

        app.playlist_key_down(Key::NumAdd, Modifiers::CTRL)
            .expect("the key applies");
        assert_eq!(app.playlist().font_size().size(), 11, "one step up");
        assert!(app.playlist().font_size().is_overridden());
    }

    /// The chosen size is persisted with the other UI state, so it survives
    /// a restart (the app keeps it in the configuration file; see
    /// `config::PLAYLIST_FONT_SIZE_KEY`).
    #[test]
    fn the_playlist_font_size_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        let mut app = test_app();
        app.set_config_path(Some(path.clone()));
        add_entries(&mut app, 4);
        assert_eq!(app.playlist_font_size(), None, "the skin's size is in use");

        app.playlist_key_down(Key::NumAdd, Modifiers::CTRL)
            .expect("the key applies");
        assert_eq!(app.playlist_font_size(), Some(14));
        app.save_config().expect("the config is written");

        let loaded = crate::config::load(Some(&path)).expect("the config loads");
        assert_eq!(loaded.playlist_font_size, Some(14));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        // A new player built from that file comes up with the same size.
        let mut restarted = test_app();
        restarted.set_playlist_font_size(loaded.playlist_font_size);
        assert_eq!(restarted.playlist().font_size().size(), 14);
        assert_eq!(restarted.playlist_font_size(), Some(14));
    }

    /// The playlist's title format (spec **S14**) is off without a setting,
    /// so the rows show exactly the text they show today.
    #[test]
    fn without_a_setting_the_row_shows_its_number_and_name() {
        let mut app = test_app();
        add_entries(&mut app, 3);
        let ids = app.playlist().visible_ids();
        let format = app.playlist_title_format().engine_format();
        assert!(!format.is_enabled(), "the format is off without a setting");
        let labels: Vec<String> = ids
            .iter()
            .map(|id| app.playlist_label(*id, &format))
            .collect();
        assert_eq!(labels, ["1. track0", "2. track1", "3. track2"]);
        // The frame is the frame of those texts: the drawing did not change
        // either.
        let frame = app.render_playlist();
        let expected = render_playlist_window(
            app.skin.skin(),
            app.playlist(),
            &labels,
            app.focused_for_playlist(),
        );
        assert_eq!(frame, expected);
    }

    /// A format the user set replaces the row text, rendered with the fields
    /// the row carries.
    #[test]
    fn a_format_the_user_set_replaces_the_row_text() {
        let mut app = test_app();
        add_entries(&mut app, 3);
        let id = app.playlist().visible_ids()[0];
        // The engine read the tags of the track it opened: the row carries
        // them, and nothing is read from the disk while a frame is painted.
        app.handle_event(&Event::TrackChanged {
            id: None,
            index: Some(0),
            metadata: Some(Box::new(TrackMetadata {
                title: Some("The Title".to_string()),
                artist: Some("The Artist".to_string()),
                ..TrackMetadata::default()
            })),
            duration_ms: None,
        });
        app.set_playlist_title_format(true, "[%artist% - ]%title%")
            .expect("the format can be used");
        let format = app.playlist_title_format().engine_format();
        assert_eq!(app.playlist_label(id, &format), "The Artist - The Title");
        // The text is the one the engine renders for those fields.
        let fields = crate::title_format::row_fields(
            &TrackSource::path("track0.mp3"),
            app.playlist_tags.metadata(id),
        );
        assert_eq!(
            app.playlist_label(id, &format),
            rustamp_ui::title_format::TitleFormat::new("[%artist% - ]%title%")
                .row_text("track0.mp3", &fields)
        );
    }

    /// A format string that cannot be read is reported, and the rows keep
    /// their plain text.
    #[test]
    fn a_format_that_cannot_be_read_leaves_the_row_text_alone_and_is_reported() {
        let mut app = test_app();
        add_entries(&mut app, 3);
        let id = app.playlist().visible_ids()[0];
        // The preferences page reports the format it cannot use.
        app.apply_prefs_action(PrefsAction::SetTitleFormat("%title".to_string()));
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("not used")),
            "{notes:?}"
        );
        // The rows keep their plain text.
        let format = app.playlist_title_format().engine_format();
        assert!(!format.is_enabled(), "the format is not used");
        assert_eq!(
            app.playlist_label(id, &format),
            "1. track0",
            "the plain text stays"
        );
        // And the reason names what is wrong with the format.
        let reason = app
            .set_playlist_title_format(true, "%title")
            .expect_err("the format is refused");
        assert!(reason.contains("field"), "{reason}");
    }

    /// The chosen title format is persisted with the other UI state, so it
    /// survives a restart (the app keeps it in the configuration file; see
    /// `config::PLAYLIST_TITLE_FORMAT_KEY`).
    #[test]
    fn the_playlist_title_format_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        let mut app = test_app();
        app.set_config_path(Some(path.clone()));
        add_entries(&mut app, 2);
        app.set_playlist_title_format(true, "%filename%")
            .expect("the format can be used");
        app.save_config().expect("the config is written");

        let loaded = crate::config::load(Some(&path)).expect("the config loads");
        assert_eq!(
            loaded.playlist_title_format,
            Some(crate::title_format::TitleFormatSetting::new(
                true,
                "%filename%"
            ))
        );
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        // A new player built from that file comes up with the same format.
        let mut restarted = test_app();
        let setting = loaded
            .playlist_title_format
            .expect("the file names a format");
        restarted
            .set_playlist_title_format(setting.enabled, &setting.format)
            .expect("the format can be used");
        assert!(restarted.playlist_title_format().enabled);
        assert_eq!(restarted.playlist_title_format().format, "%filename%");
    }

    /// A long format does not change the row height, and its text does not
    /// leave the row's band: the row cuts the text off at its width.
    #[test]
    fn a_long_format_stays_inside_the_row_band() {
        let mut app = test_app();
        add_entries(&mut app, 8);
        let before = app.render_playlist();
        let pitch = app.playlist().row_pitch();
        let rows = app.playlist().visible_rows();
        app.set_playlist_title_format(true, &"x".repeat(2_000))
            .expect("the format can be used");
        let after = app.render_playlist();
        assert_eq!(after.width(), before.width(), "the frame is as wide");
        assert_eq!(after.height(), before.height(), "the frame is as tall");
        assert_eq!(
            app.playlist().row_pitch(),
            pitch,
            "the row height did not change"
        );
        assert_eq!(
            app.playlist().visible_rows(),
            rows,
            "the rows did not change"
        );
        // Every pixel the long format changed is inside a row: the text is
        // cut off at the row's width, so it cannot reach the rails or the
        // scrollbar.
        for y in 0..after.height() as i32 {
            for x in 0..after.width() as i32 {
                if before.pixel(x, y) == after.pixel(x, y) {
                    continue;
                }
                let inside_a_row = (0..rows).any(|row| {
                    rustamp_ui::playlist_window::row_rect(row, pitch).is_some_and(|rect| {
                        rect.x <= x && x < rect.right() && rect.y <= y && y < rect.bottom()
                    })
                });
                assert!(inside_a_row, "the pixel at ({x}, {y}) is outside the rows");
            }
        }
    }

    #[test]
    fn misc_requests_the_native_popup_without_emitting_a_command() {
        let mut app = test_app();
        add_entries(&mut app, 3);
        assert!(!app.take_playlist_misc_menu());
        let commands = app
            .playlist_mouse_down(112, 92, Modifiers::NONE)
            .expect("MISC applies");
        assert!(commands.is_empty());
        assert!(app.take_playlist_misc_menu());
        assert!(!app.take_playlist_misc_menu());
    }

    #[test]
    fn every_misc_id_uses_existing_menu_dispatch() {
        let mut app = test_app();
        add_entries(&mut app, 3);
        let menu = rustamp_ui::menu::native_playlist_misc_menu();
        let entries = app.playlist().visible_ids();
        let selection = app.playlist().selected_ids();
        // "Generate HTML playlist" is the editor's `Ctrl+Alt+G` row: the same
        // command id, so the menu asks for the document like the key does.
        assert_eq!(
            app.activate_menu_choice(menu.choice_for_id(40292).unwrap())
                .unwrap(),
            None
        );
        assert!(app.request_html_playlist, "the document is requested");
        assert!(app.drain_notes().is_empty());
        assert_eq!(app.engine().queue().len(), 3);
        assert_eq!(app.playlist().visible_ids(), entries);
        assert_eq!(app.playlist().selected_ids(), selection);

        // "Read extended info on selection" is the editor's `Ctrl+Alt+E` row:
        // the same command id, so the menu reads the tags of the selection
        // like the key does.
        let first = app.entry_ids[0];
        app.submit(Command::Select(SelectOp::Set(vec![first])))
            .unwrap();
        assert_eq!(
            app.activate_menu_choice(menu.choice_for_id(40293).unwrap())
                .unwrap(),
            None
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::ExtendedInfo));
        let notes = app.drain_notes();
        assert!(
            notes.iter().all(|note| !note.contains("not implemented")),
            "{notes:?}"
        );
        assert_eq!(app.engine().queue().len(), 3);
        assert_eq!(app.playlist().visible_ids(), entries);
        assert_eq!(app.playlist().selected_ids(), vec![first]);
    }

    #[test]
    fn scrollbar_arrows_page_through_the_list() {
        // S06 §3: one arrow step is one page of four rows.
        let mut app = test_app();
        add_entries(&mut app, 60);
        app.playlist_mouse_down(263, 87, Modifiers::NONE)
            .expect("down applies");
        assert_eq!(app.playlist().first_visible(), 4);
        app.playlist_mouse_down(263, 81, Modifiers::NONE)
            .expect("up applies");
        assert_eq!(app.playlist().first_visible(), 0);
    }

    #[test]
    fn the_title_bar_has_no_window_menu_button() {
        // S06 §1: a (6,3) click does nothing.
        let mut app = test_app();
        add_entries(&mut app, 3);
        let commands = app
            .playlist_mouse_down(6, 3, Modifiers::NONE)
            .expect("click applies");
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
        let commands = app
            .playlist_mouse_down(264, 3, Modifiers::NONE)
            .expect("close applies");
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

    /// The shade button is the title-bar button at (254,3) (S01 §4,
    /// provisional rect). One click collapses the editor to its 14 px title
    /// strip, the next one expands it again (S01 §1).
    #[test]
    fn the_title_bar_shade_button_collapses_and_expands_the_editor() {
        let mut app = test_app();
        assert!(!app.playlist_shaded());
        assert_eq!(app.playlist_frame_size(), (275, 116));

        let commands = app
            .playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        assert_eq!(
            commands,
            vec![Command::ToggleWindowShade(WindowId::Playlist)]
        );
        assert!(app.playlist_shaded(), "the editor is in windowshade form");
        assert_eq!(app.playlist_frame_size(), (275, 14), "one row tall");
        assert!(app.playlist_open(), "shading does not hide the editor");

        // The same button expands it again.
        let commands = app
            .playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        assert_eq!(
            commands,
            vec![Command::ToggleWindowShade(WindowId::Playlist)]
        );
        assert!(!app.playlist_shaded());
        assert_eq!(app.playlist_frame_size(), (275, 116));
        assert!(app.drain_notes().is_empty());
    }

    /// Only the shade button toggles: the rest of the title bar is the
    /// window's drag area, the (6,3) spot has no button at all (S06 §1) and
    /// the list and the bottom bar are not the shade.
    #[test]
    fn a_click_outside_the_shade_button_never_toggles_the_editor() {
        for (x, y) in [(6, 3), (100, 5), (254, 12), (254, 20), (264, 3)] {
            let mut app = test_app();
            let commands = app
                .playlist_mouse_down(x, y, Modifiers::NONE)
                .expect("click applies");
            assert!(
                !commands.contains(&Command::ToggleWindowShade(WindowId::Playlist)),
                "({x},{y}) must not shade the editor: {commands:?}"
            );
            assert!(!app.playlist_shaded(), "({x},{y}) must not shade");
        }
    }

    /// A shaded editor is a 14 px title strip: it has no list and no bottom
    /// bar, so only its title-bar buttons answer a click.
    #[test]
    fn a_shaded_editor_answers_only_its_title_bar() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        assert!(app.playlist_shaded());

        // The list and the bottom bar are gone with the body.
        assert!(
            app.playlist_mouse_down(60, 28, Modifiers::NONE)
                .expect("click")
                .is_empty()
        );
        assert!(
            app.playlist_mouse_down(24, 92, Modifiers::NONE)
                .expect("ADD")
                .is_empty()
        );
        assert!(app.playlist().selected_ids().is_empty());
        assert!(app.drain_notes().is_empty());

        // The shade button still expands it.
        let commands = app
            .playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        assert_eq!(
            commands,
            vec![Command::ToggleWindowShade(WindowId::Playlist)]
        );
        assert!(!app.playlist_shaded());
    }

    /// The docked stack keeps its edge connections when the editor is shaded:
    /// the editor becomes one row tall, the windows above it stay where they
    /// are, and the main window's own shade still re-flows the editor
    /// (S05 §3.3, §5).
    #[test]
    fn shading_the_editor_makes_it_one_row_tall_without_moving_its_neighbours() {
        let mut app = test_app();
        let before = app.docked_stack();
        assert_eq!(before.playlist.size(), (275, 116));

        app.playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        let shaded = app.docked_stack();
        assert_eq!(shaded.playlist.size(), (275, 14), "one row tall");
        assert_eq!(
            (shaded.playlist.x, shaded.playlist.y),
            (before.playlist.x, before.playlist.y),
            "the editor keeps the equalizer's bottom edge"
        );
        assert_eq!(
            (shaded.equalizer, shaded.main),
            (before.equalizer, before.main),
            "the windows above the editor do not move"
        );

        // The main window's shade still re-flows the shaded editor up by
        // 102 px (S05 §3.3).
        app.submit(Command::ToggleWindowShade(WindowId::Main))
            .expect("shade toggles");
        let with_shaded_main = app.docked_stack();
        assert_eq!(with_shaded_main.playlist.y, 159);
        assert_eq!(with_shaded_main.playlist.size(), (275, 14));
    }

    /// The shaded editor paints the skin's 14 px band, so the frame the app
    /// renders and the frame size the OS window asks for agree.
    #[test]
    fn the_shaded_editor_renders_the_skins_shade_band() {
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.playlist_mouse_down(254, 3, Modifiers::NONE)
            .expect("shade applies");
        let image = app.render_playlist();
        assert_eq!((image.width(), image.height()), (275, 14));
        assert_eq!(
            (image.width(), image.height()),
            app.playlist_frame_size(),
            "the painted frame and the OS client area agree"
        );
    }

    #[test]
    fn rendering_produces_a_frame_of_the_playlist_size() {
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("stage a selection");
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
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("stage a selection");
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
    fn alt_delete_removes_the_dead_files_and_ctrl_shift_delete_clears() {
        // S03 §2: Alt+Delete (`40222`) removes all dead files (RFC 0018);
        // Ctrl+Shift+Delete and Ctrl+N both clear the playlist (`40214`).
        // The entries point into a temp dir that holds one real file, so the
        // outcome does not depend on the working directory.
        let mut app = test_app();
        let dir = tempfile::tempdir().expect("a temp dir");
        let alive = dir.path().join("alive.mp3");
        std::fs::write(&alive, b"").expect("write the fixture");
        let gone = dir.path().join("gone.mp3");
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(alive), MediaLocation::File(gone)],
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        assert_eq!(app.playlist().len(), 2);

        let outcome = app
            .playlist_key_down(Key::Delete, Modifiers::ALT)
            .expect("Alt+Delete applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::RemoveDeadFiles]))
        );
        assert_eq!(app.playlist().len(), 1);
        assert_eq!(app.engine().queue().len(), 1);

        // Ctrl+Shift+Delete clears what is left.
        let outcome = app
            .playlist_key_down(Key::Delete, Modifiers::CTRL | Modifiers::SHIFT)
            .expect("Ctrl+Shift+Delete applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Clear]))
        );
        assert!(app.playlist().is_empty());

        // Ctrl+N is the second chord for the same command id.
        add_entries(&mut app, 2);
        assert_eq!(app.playlist().len(), 2);
        let outcome = app
            .playlist_key_down(Key::Letter(b'N'), Modifiers::CTRL)
            .expect("Ctrl+N applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Clear]))
        );
        assert!(app.playlist().is_empty());
        assert!(app.engine().queue().is_empty());
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
        // The playlist's own Ctrl+A shadows the main and global table's
        // `40019` "Toggle always on top".
        assert!(!app.always_on_top());
        app.playlist_key_down(Key::Letter(b'I'), Modifiers::CTRL)
            .expect("Ctrl+I applies");
        assert!(app.playlist().selected_ids().is_empty());
    }

    #[test]
    fn the_add_shortcuts_open_the_matching_media_face() {
        // S03 §2: the playlist editor's own add commands — `1032` "Add
        // file(s)", `1036` "Add directory" and `1039` "Add location".
        let mut app = test_app();
        for (key, modifiers, kind) in [
            (Key::Letter(b'L'), Modifiers::NONE, OpenMediaKind::Files),
            (Key::Letter(b'L'), Modifiers::CTRL, OpenMediaKind::Location),
            (
                Key::Letter(b'L'),
                Modifiers::SHIFT,
                OpenMediaKind::Directory,
            ),
            (Key::Insert, Modifiers::NONE, OpenMediaKind::Directory),
            (Key::Insert, Modifiers::CTRL, OpenMediaKind::Files),
            (Key::Insert, Modifiers::SHIFT, OpenMediaKind::Location),
            (Key::Numpad(0), Modifiers::NONE, OpenMediaKind::Files),
            (Key::Numpad(0), Modifiers::CTRL, OpenMediaKind::Location),
        ] {
            let outcome = app
                .playlist_key_down(key, modifiers)
                .unwrap_or_else(|error| panic!("{key:?} {modifiers:?}: {error}"));
            assert_eq!(
                outcome,
                Some(ShortcutOutcome::Commands(vec![Command::OpenMedia {
                    kind,
                    play: false
                }])),
                "{key:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn ctrl_o_opens_the_playlist_and_ctrl_r_reverses_the_list() {
        // S03 §2: Ctrl+O is `40202` "Open playlist", Ctrl+R is `40213`
        // "Reverse the list".
        let mut app = test_app();
        let items = vec![
            MediaLocation::File(PathBuf::from("a.mp3")),
            MediaLocation::File(PathBuf::from("b.mp3")),
            MediaLocation::File(PathBuf::from("c.mp3")),
        ];
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        let outcome = app
            .playlist_key_down(Key::Letter(b'O'), Modifiers::CTRL)
            .expect("Ctrl+O applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::OpenPlaylist]))
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));

        let outcome = app
            .playlist_key_down(Key::Letter(b'R'), Modifiers::CTRL)
            .expect("Ctrl+R applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Reverse]))
        );
        assert_eq!(
            app.entry_ids,
            vec![EntryId(2), EntryId(1), EntryId(0)],
            "the rows turn around, the entries survive"
        );
        assert_eq!(
            app.playlist().visible_ids(),
            vec![EntryId(2), EntryId(1), EntryId(0)]
        );
    }

    #[test]
    fn alt_3_opens_the_file_info_and_alt_i_bookmarks_the_selection() {
        // S03 §2: Alt+3 is `40208` "File info", Alt+I is `40319`
        // "Bookmark the selected item(s)".
        let mut app = test_app();
        let outcome = app
            .playlist_key_down(Key::Digit(3), Modifiers::ALT)
            .expect("Alt+3 applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::OpenFileInfo]))
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::FileInfo));

        let outcome = app
            .playlist_key_down(Key::Letter(b'I'), Modifiers::ALT)
            .expect("Alt+I applies");
        assert_eq!(outcome, Some(ShortcutOutcome::Commands(Vec::new())));
    }

    #[test]
    fn ctrl_f4_closes_the_playlist_editor() {
        // S03 §2: Ctrl+F4 is `40224` "Close the playlist editor".
        let mut app = test_app();
        assert!(app.playlist_open(), "the editor starts open");
        let outcome = app
            .playlist_key_down(Key::F(4), Modifiers::CTRL)
            .expect("Ctrl+F4 applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::ToggleWindow(
                WindowId::Playlist
            )]))
        );
        assert!(!app.playlist_open());
    }

    #[test]
    fn ctrl_w_toggles_the_editors_windowshade() {
        // S03 §2: Ctrl+W is `40064` "Toggle windowshade". The editor's shade
        // is its own (S01 §1), so the row toggles the editor and not the main
        // window.
        let mut app = test_app();
        assert!(!app.playlist_shaded());
        let outcome = app
            .playlist_key_down(Key::Letter(b'W'), Modifiers::CTRL)
            .expect("Ctrl+W applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::ToggleWindowShade(
                WindowId::Playlist
            )]))
        );
        assert!(app.playlist_shaded());
        assert!(!app.state().windowshade, "the main window keeps its shade");
    }

    #[test]
    fn ctrl_s_asks_the_presenter_for_the_save_dialog() {
        // S03 §2: Ctrl+S is `40204` "Save playlist"; the presenter opens the
        // native save dialog and sends the chosen path.
        let mut app = test_app();
        let outcome = app
            .playlist_key_down(Key::Letter(b'S'), Modifiers::CTRL)
            .expect("Ctrl+S applies");
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(app.request_save_playlist);
    }

    #[test]
    fn the_shared_rows_reach_the_shared_path() {
        // S03 §2 shares most of its rows with S03 §1/§4; they run against the
        // same machine, so the command is the one the main window sends.
        let mut app = test_app();
        let outcome = app
            .playlist_key_down(Key::Letter(b'B'), Modifiers::NONE)
            .expect("B applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::Next]))
        );

        let outcome = app
            .playlist_key_down(Key::F(1), Modifiers::NONE)
            .expect("F1 applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::OpenAbout]))
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::About));

        // Ctrl+Tab moves the focus to the next player window (the row is
        // provisional in S03 §2, the command exists).
        let outcome = app
            .playlist_key_down(Key::Tab, Modifiers::CTRL)
            .expect("Ctrl+Tab applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::FocusNextWindow]))
        );

        // Ctrl+T toggles the elapsed/remaining time of the main window.
        let outcome = app
            .playlist_key_down(Key::Letter(b'T'), Modifiers::CTRL)
            .expect("Ctrl+T applies");
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(app.state().time_remaining);
    }

    #[test]
    fn alt_up_and_alt_down_move_the_selected_block() {
        // S03 §2: Alt+Up is `40215` and Alt+Down is `40216` "Move the
        // selected entry up/down" (provisional).
        let mut app = test_app();
        add_entries(&mut app, 4);
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1), EntryId(2)])))
            .expect("Select applies");
        let outcome = app
            .playlist_key_down(Key::Up, Modifiers::ALT)
            .expect("Alt+Up applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::MoveEntries {
                ids: vec![EntryId(1), EntryId(2)],
                to: 0,
            }]))
        );
        assert_eq!(
            app.entry_ids,
            vec![EntryId(1), EntryId(2), EntryId(0), EntryId(3)]
        );

        let outcome = app
            .playlist_key_down(Key::Down, Modifiers::ALT)
            .expect("Alt+Down applies");
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::MoveEntries {
                ids: vec![EntryId(1), EntryId(2)],
                to: 1,
            }]))
        );
        assert_eq!(
            app.entry_ids,
            vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)]
        );
        // The selection follows the rows it moved.
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1), EntryId(2)]);
    }

    #[test]
    fn the_edit_entry_row_does_not_fall_through_to_the_main_table() {
        // S03 §2's `40255` "Edit the playlist entry" has its own row: the
        // dialog's model is in `rustamp-ui`'s `dialogs` module and the app
        // half is in `crate::dialogs`. The row must not fall through to the
        // main table's `40186` "Toggle easy move".
        let mut app = test_app();
        let easy_move = app.easy_move();
        let outcome = app
            .playlist_key_down(Key::Letter(b'E'), Modifiers::CTRL)
            .expect("Ctrl+E applies");
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert_eq!(
            app.easy_move(),
            easy_move,
            "the main table's Ctrl+E must not run"
        );
        // With nothing selected the row says so and opens no box.
        assert!(!app.dialog_is_open(rustamp_ui::DialogKind::EditEntry));
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("nothing is selected")),
            "{notes:?}"
        );
        assert!(
            !notes.iter().any(|note| note.contains("40255")),
            "{notes:?}"
        );
    }

    /// A copy of the tagged MP3 fixture, in a temporary directory.
    ///
    /// The fixture lives in the decoders' test data (`sine.mp3`); it is
    /// copied, never written to.
    fn tagged_mp3(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../rustamp-decoders/tests/fixtures/sine.mp3");
        let path = dir.path().join(name);
        std::fs::copy(&fixture, &path).expect("copy the tagged fixture");
        path
    }

    /// Adds one entry to the queue and returns its id.
    fn add_entry(app: &mut App, location: MediaLocation) -> EntryId {
        app.submit(Command::AddItems {
            items: vec![location],
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        *app.entry_ids.last().expect("the entry has an id")
    }

    /// Presses the playlist's `Ctrl+Alt+E` row (S03 §2 `40293`).
    fn read_extended_info(app: &mut App) -> Option<ShortcutOutcome> {
        app.playlist_key_down(Key::Letter(b'E'), Modifiers::CTRL | Modifiers::ALT)
            .expect("Ctrl+Alt+E applies")
    }

    #[test]
    fn the_extended_info_row_shows_the_tags_of_the_selected_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        let id = add_entry(&mut app, MediaLocation::File(path.clone()));
        app.submit(Command::Select(SelectOp::Set(vec![id])))
            .unwrap();

        let outcome = read_extended_info(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::ExtendedInfo));
        let lines = app.open_dialog_lines().expect("the box has lines");
        let text = lines.join("\n");
        assert!(text.contains("1 entry selected"), "{text}");
        assert!(text.contains(&path.display().to_string()), "{text}");
        assert!(text.contains("Title: Sine Title"), "{text}");
        assert!(text.contains("Artist: Test Artist"), "{text}");
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn the_extended_info_row_summarises_a_multi_selection() {
        // Own choice: a multi-selection is summarised — the box names how
        // many entries are selected and shows the tags of the **first** one.
        let dir = tempfile::tempdir().expect("a temp dir");
        let first = tagged_mp3(&dir, "first.mp3");
        let second = tagged_mp3(&dir, "second.mp3");
        let mut app = test_app();
        let a = add_entry(&mut app, MediaLocation::File(first.clone()));
        let b = add_entry(&mut app, MediaLocation::File(second.clone()));
        app.submit(Command::Select(SelectOp::Set(vec![a, b])))
            .unwrap();

        read_extended_info(&mut app);
        let lines = app.open_dialog_lines().expect("the box has lines");
        assert!(lines[0].contains("2 entries selected"), "{lines:?}");
        assert!(lines[0].contains("showing the first"), "{lines:?}");
        assert!(lines[1].contains(&first.display().to_string()), "{lines:?}");
        assert!(
            lines.iter().any(|line| line == "Title: Sine Title"),
            "{lines:?}"
        );
    }

    #[test]
    fn the_extended_info_row_reports_a_missing_file_and_a_stream() {
        // A file that cannot be read and a stream are reported, not a panic.
        let mut app = test_app();
        let missing = add_entry(&mut app, MediaLocation::File(PathBuf::from("gone.mp3")));
        let stream = add_entry(
            &mut app,
            MediaLocation::Url("http://example.com/stream".to_string()),
        );

        // The missing file: the box names the error.
        app.submit(Command::Select(SelectOp::Set(vec![missing])))
            .unwrap();
        read_extended_info(&mut app);
        let text = app
            .open_dialog_lines()
            .expect("the box has lines")
            .join("\n");
        assert!(text.contains("could not be read"), "{text}");
        assert!(text.contains("gone.mp3"), "{text}");

        // The stream: it has no local file, so it has no tags.
        app.submit(Command::Select(SelectOp::Set(vec![stream])))
            .unwrap();
        read_extended_info(&mut app);
        let text = app
            .open_dialog_lines()
            .expect("the box has lines")
            .join("\n");
        assert!(text.contains("a stream has no local file"), "{text}");
        assert!(text.contains("http://example.com/stream"), "{text}");
    }

    #[test]
    fn the_extended_info_row_with_nothing_selected_says_so_and_opens_nothing() {
        let mut app = test_app();
        add_entries(&mut app, 2);
        let outcome = read_extended_info(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(!app.dialog_is_open(rustamp_ui::DialogKind::ExtendedInfo));
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("nothing is selected")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_extended_info_row_is_not_unimplemented_any_more() {
        // The row is bound now: the app neither notes a gap nor lets the
        // chord fall through to another table.
        let mut app = test_app();
        add_entries(&mut app, 1);
        read_extended_info(&mut app);
        let notes = app.drain_notes();
        assert!(
            !notes.iter().any(|note| note.contains("no command yet")),
            "{notes:?}"
        );
        assert!(
            !notes.iter().any(|note| note.contains("40293")),
            "{notes:?}"
        );
    }

    #[test]
    fn other_playlist_keys_take_the_shared_path_and_leave_the_list_alone() {
        // S03 §2: Up is a provisional playlist row with no measured effect, so
        // it reaches the shared shortcut path and does not move the selection.
        let mut app = test_app();
        add_entries(&mut app, 6);
        app.playlist_mouse_down(60, 28, Modifiers::NONE)
            .expect("stage a selection");
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
