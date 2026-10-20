// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Simple modal dialogs: About, file info, jump-to (S03 actions), the
//! load-playlist dialog (S06 §4 LIST OPTS, RFC 0010), the open-media dialogs
//! (S03 ids 40029/40185/40187, RFC 0005 `Command::OpenMedia`; the playlist
//! ADD button reaches the files face), the skin browser
//! (S03 id 40219, RFC 0005) and the preferences frame (S03 ids 40012, 40191,
//! 40317, RFC 0005 §7).
//!
//! The headless state ([`rustamp_ui::DialogManager`]) lives in `rustamp-ui`;
//! this module is the app glue: opening a dialog for a [`Command`], closing
//! it, typing into the jump box, moving the skin browser's selection, and
//! assembling each dialog's body lines. Everything here is headless: the OS
//! window itself is created by [`crate::window`], which calls back into the
//! [`App`] methods below.
//!
//! What each dialog shows (kept simple on purpose — no audited spec covers
//! dialog contents; S04 is not started, so sizes and lines are own choices,
//! pinned by tests):
//!
//! * About: the program name, the crate version and the license line.
//! * File info: the current queue entry's display name (or "no track
//!   loaded"), the stream line (bitrate / sample rate / channels) and the
//!   position/duration line, all from the engine snapshot.
//! * Jump to time: the prompt plus the typed input; `Enter` confirms it to
//!   [`Command::Seek`].
//! * Jump to file (S03 id 40194): a search box over the playlist. It shows
//!   the typed search text, the entries whose display text contains it
//!   (ignoring case) in a scrollable window with the selected one marked,
//!   and a count line. `Up`/`Down` move the selection, `Enter` plays the
//!   selected entry ([`Command::PlayEntry`]) and closes the box; with no
//!   match the box stays open so the search can be corrected. The list
//!   layout, sorting and wording are unmeasured (S04 open), so the row
//!   shape and the count line are own choices.
//! * Load playlist: the spec'd title plus an entry field (or staged pick) for
//!   the playlist file. `Enter` confirms and loads the chosen playlist file
//!   through [`App::load_playlist_file`], enqueuing its surviving tracks in
//!   order, beginning playback, and surfacing any skipped entries. With
//!   nothing chosen `Enter` stays consumed and the dialog stays open.
//! * Open media: a kind-specific face — the prompt plus the entry field for
//!   the files and directory faces, the staged-pick summary when picks were
//!   staged, or the typed URL for the location face, then a one-line hint.
//!   The field takes the OS text of each key press ([`App::dialog_text`]),
//!   so a full path is typeable, and files dropped from the shell become
//!   picks ([`App::dialog_drop_file`]); a long field shows its tail. There
//!   is no OS picker and no clipboard paste (both would need a new Win32
//!   binding). This is the target of the main window's Eject and the
//!   playlist's ADD (BETA-03). Confirming adds the
//!   picks (`Enter` submits the resulting [`Command::AddItems`] and closes
//!   the dialog); with nothing to confirm the dialog stays open. The
//!   reference's layout, filters and wording are unmeasured, so prompts are
//!   plain own choices. The `play` flag rides through untouched (RFC 0005
//!   O5: the playlist ADD path's `false` is unmeasured).
//! * Skin browser: the skins the app can load today (the built-in default,
//!   the skins of the folder beside the configuration and the current skin
//!   when it is not one of them), with the selected row marked. `Enter` loads
//!   the selected skin through the existing skin path (`Command::LoadSkin` for
//!   a file row, the built-in loader otherwise) and closes the dialog.
//! * Preferences: the frame shows the requested page's own-choice name
//!   (`rustamp-prefs`) and nothing else — the reference's page contents are
//!   unmeasured (S04 open), so there are no controls to draw, type into or
//!   confirm. `Enter` stays consumed and keeps the dialog open; `Escape` (or
//!   the window's close button) closes it.
//! * Extended info (S03 §2 `40293` "Read extended info on selection"): the
//!   tags of the selected playlist entries. The app reads them when the
//!   action runs (the tags are read once, not per frame) and the box shows
//!   the snapshot: the count of the selection, the entry the tags belong to
//!   (the first selected one, in playlist order) and its tag fields — or the
//!   reason it has none: a stream has no local file, the file could not be
//!   read, or it carries no tags. S03 §2 gives the id and the chord only, so
//!   the content is an own choice (see
//!   [`rustamp_ui::dialogs::ExtendedInfo`]).
//! * Edit entry (S03 §2 `40255` "Edit the playlist entry"): the **tags of
//!   one** selected playlist entry, one field per tag. The app reads the
//!   tags when the action runs, opens the box with them and writes the
//!   fields the user changed back into the file with `rustamp-meta`. A
//!   field the user **cleared** is removed from the file instead of being
//!   written as an empty value, and a value the file keeps in a second tag
//!   (an ID3v1 tag next to an ID3v2 tag) is reported rather than hidden.
//!   A stream has no local file and therefore no tags to edit: the action
//!   says so and opens nothing, and so does an empty selection. A field
//!   that does not validate keeps the box open, so the user can fix it;
//!   `Escape` closes the box and writes nothing. S03 §2 gives the id and
//!   the chord only, so the fields and the validation are an own choice
//!   (see [`rustamp_ui::dialogs::EditEntryDialog`]).

use std::path::PathBuf;

use rustamp_audio::TrackSource;
use rustamp_core::command::{Command, JumpTarget, OpenMediaKind, PrefsPage};
use rustamp_core::meta::TrackMetadata;
use rustamp_gfx::Image;
use rustamp_meta::TagField;
use rustamp_platform::{Bind, PluginRow, PrefsAction, PrefsNode, PrefsSettings};
use rustamp_plugin_abi::PluginKind;
use rustamp_prefs::page_name;
use rustamp_ui::{
    DialogKind, EditEntryChanges, EditEntryDialog, EditEntryOutcome, ExtendedInfo, Key, Modifiers,
    SkinEntry, dialog_for_command, edit_entry_lines, extended_info_lines, render_dialog,
    skin_entries,
};

use crate::app::App;

/// The edit-entry session: the file whose tags the dialog edits and the
/// dialog's model.
///
/// The session lives as long as the dialog is open. The file is named here
/// because the dialog itself only knows the tag fields: writing the changed
/// fields back into the file is the app's job, and the app needs the path
/// for it.
pub(crate) struct EditEntrySession {
    /// The file whose tags are edited.
    pub(crate) path: PathBuf,
    /// The dialog's model: the fields, the focus and the validation.
    pub(crate) dialog: EditEntryDialog,
}

impl App {
    /// Opens the dialog `command` asks for, when it asks for one the app now
    /// implements (About, file info, either jump box, load playlist, the
    /// three open-media faces, skin browser, preferences).
    ///
    /// Returns the opened dialog, or `None` when the command opens no
    /// implemented dialog (the caller keeps its existing note path then).
    pub fn open_dialog_for(&mut self, command: &Command) -> Option<DialogKind> {
        let dialog = dialog_for_command(command)?;
        self.dialogs.open(dialog);
        // The new dialog replaces the open one, so the edit-entry session
        // (the file it edits) goes with it.
        self.edit_entry = None;
        // The browser moves its selection with the arrow keys, and without the
        // row count a held Down would overrun the end and a following Up would
        // appear dead. Tell the manager how many rows it is looking at.
        if dialog == DialogKind::SkinBrowser {
            self.dialogs.set_skin_rows(self.browser_entries().len());
        }
        self.sync_jump_rows();
        Some(dialog)
    }

    /// The queue indices the jump-to-file box lists: every entry whose
    /// display text contains the search text (ignoring case), in playlist
    /// order. An empty search lists the whole playlist.
    pub(crate) fn jump_file_matches(&self) -> Vec<usize> {
        let needle = self.dialogs.jump_text().to_lowercase();
        self.engine()
            .queue()
            .iter()
            .enumerate()
            .filter(|(_, source)| {
                needle.is_empty() || source.display().to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Tells the dialog manager how many results the jump-to-file box has,
    /// so its selection saturates at the ends. A no-op for other dialogs.
    fn sync_jump_rows(&mut self) {
        if self.dialog_is_open(DialogKind::Jump(JumpTarget::File)) {
            let rows = self.jump_file_matches().len();
            self.dialogs.set_jump_rows(rows);
        }
    }

    /// The jump-to-file box's body: the search field, then a window of
    /// [`JUMP_FILE_ROWS`](rustamp_ui::JUMP_FILE_ROWS) matches that keeps the
    /// selected row (marked `"> "`) in view, then a count line.
    fn jump_file_lines(&self) -> Vec<String> {
        let matches = self.jump_file_matches();
        let selected = self.dialogs.jump_index();
        let rows = rustamp_ui::JUMP_FILE_ROWS;
        let first = selected.saturating_sub(rows - 1);
        let queue = self.engine().queue();
        let mut lines = vec![format!("Search: {}", self.dialogs.jump_text())];
        if matches.is_empty() {
            lines.push(if queue.is_empty() {
                "(the playlist is empty)".to_string()
            } else {
                "(no match)".to_string()
            });
        }
        for (row, &index) in matches.iter().enumerate().skip(first).take(rows) {
            let marker = if row == selected { "> " } else { "  " };
            let prefix = format!("{marker}{}. ", index + 1);
            let text = queue.get(index).map(|source| source.display());
            // The row's own prefix takes cells of the line, so the entry's
            // text gets only what is left; otherwise the tail the user
            // searched for would be clipped away by the dialog's edge.
            let cells = jump_row_cells(prefix.chars().count());
            lines.push(format!(
                "{prefix}{}",
                tail_within(&text.unwrap_or_default(), cells)
            ));
        }
        lines.push(format!("{} of {} entries", matches.len(), queue.len()));
        lines
    }

    /// Confirms the jump-to-file box: plays the selected match and closes
    /// the dialog. With no match the dialog stays open so the search can be
    /// corrected.
    fn confirm_jump_file(&mut self) {
        let matches = self.jump_file_matches();
        let Some(&index) = matches.get(self.dialogs.jump_index()) else {
            return;
        };
        let Some(&id) = self.entry_ids.get(index) else {
            return;
        };
        let _ = self.submit(Command::PlayEntry(id));
        self.dialogs.close_any();
    }

    /// The open dialog, if any.
    pub fn dialog_open_kind(&self) -> Option<DialogKind> {
        self.dialogs.open_kind()
    }

    /// The body lines of the open dialog, if any.
    pub fn open_dialog_lines(&self) -> Option<Vec<String>> {
        self.dialogs.open_kind().map(|d| self.dialog_body(d))
    }

    /// Whether `dialog` is open.
    pub fn dialog_is_open(&self, dialog: DialogKind) -> bool {
        self.dialogs.is_open(dialog)
    }

    /// The preferences dialog's current page, when it is open.
    pub fn prefs_page(&self) -> Option<PrefsPage> {
        self.dialogs.prefs_page()
    }

    /// Switches the open preferences dialog to `page`. Returns whether a
    /// preferences dialog was open (and now shows `page`).
    pub fn select_prefs_page(&mut self, page: PrefsPage) -> bool {
        self.dialogs.select_prefs_page(page)
    }

    /// Closes `dialog` when it is open.
    pub fn close_dialog(&mut self, dialog: DialogKind) {
        self.dialogs.close(dialog);
        if dialog == DialogKind::EditEntry {
            self.edit_entry = None;
        }
    }

    /// Closes whatever dialog is open, if any. Returns whether one was open.
    pub fn close_open_dialog(&mut self) -> bool {
        let open = self.dialogs.open_kind().is_some();
        self.dialogs.close_any();
        self.edit_entry = None;
        open
    }

    /// A key press while a dialog is focused.
    ///
    /// `Enter` confirms the jump-to-time box (submitting the resulting
    /// [`Command::Seek`]), the skin browser's selected skin, the
    /// load-playlist dialog (loading the file and closing the dialog), the
    /// open-media dialog (submitting the resulting [`Command::AddItems`] and
    /// closing it; with nothing to confirm the dialog stays open) or the
    /// edit-entry box (writing the changed tags into the file);
    /// `Escape` closes the open dialog; digits type into the jump box (and
    /// letters/digits/spaces into the open-media and load-playlist fields) and
    /// arrows move the skin selection.
    /// Returns `true` when the key was consumed.
    ///
    /// The modifiers belong to the key: the edit-entry box moves its focus
    /// with `Tab` and `Shift`+`Tab`, and the two are the same key with a
    /// modifier the box has to see.
    pub fn dialog_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if self.edit_entry_session().is_some() {
            return self.edit_entry_key(key, modifiers);
        }
        let is_load_playlist = self.dialog_is_open(DialogKind::LoadPlaylist);
        if is_load_playlist {
            match key {
                Key::Digit(digit) | Key::Numpad(digit) => {
                    let ch = (b'0' + (digit % 10)) as char;
                    return self.dialogs.type_text(&ch.to_string());
                }
                Key::Letter(letter) => {
                    let ch = letter as char;
                    return self.dialogs.type_text(&ch.to_string());
                }
                Key::Space => {
                    return self.dialogs.type_text(" ");
                }
                _ => {}
            }
        }
        let consumed = self.dialogs.key(key);
        if !consumed {
            return false;
        }
        self.sync_jump_rows();
        match key {
            Key::Enter => {
                if self.dialog_is_open(DialogKind::Jump(JumpTarget::File)) {
                    self.confirm_jump_file();
                } else if is_load_playlist {
                    if self.confirm_load_playlist() {
                        self.dialogs.close_any();
                    }
                } else if let Some(seek) = self.dialogs.confirm_jump_time() {
                    // A failed submit still consumed the key: the dialog stays
                    // open so the user can retry. On success, close the dialog so
                    // it does not linger or double-apply.
                    if self.submit(seek).is_ok() {
                        self.dialogs.close_any();
                    }
                } else if self.dialogs.open_kind() == Some(DialogKind::SkinBrowser) {
                    self.confirm_skin_browser();
                } else if let Some(add) = self.dialogs.confirm_open_media() {
                    // A failed submit still consumed the key; the dialog closes
                    // only on a real confirm so an empty pick cannot trap the
                    // player and a confirmed one does not linger.
                    if self.submit(add).is_ok() {
                        self.dialogs.close_any();
                    }
                }
            }
            Key::Escape => {
                self.dialogs.close_any();
            }
            _ => {}
        }
        true
    }

    /// The edit-entry session, when the edit-entry box is the open dialog.
    ///
    /// A session belongs to the box that opened it: another dialog replaces
    /// the box, and the session goes with it, so a stale one never takes a
    /// key press or a typed text that belongs to the dialog in front.
    fn edit_entry_session(&self) -> Option<&EditEntrySession> {
        if self.dialog_is_open(DialogKind::EditEntry) {
            self.edit_entry.as_ref()
        } else {
            None
        }
    }

    /// A key press in the edit-entry box (S03 §2 `40255`).
    ///
    /// The box owns the fields: `Tab` and `Shift`+`Tab` move the focus, the
    /// text keys type into the focused field, `Backspace` deletes, `Enter`
    /// saves and `Escape` cancels. The app's part is the save: the changed
    /// fields go into the file's tags, and the box closes when that worked.
    ///
    /// A field that does not validate keeps the box open (the message is in
    /// the box's last line) and a failed write keeps it open too, so the
    /// user can try again. Returns whether the key was consumed.
    fn edit_entry_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let outcome = match self.edit_entry.as_mut() {
            Some(session) => session.dialog.key(key, modifiers),
            None => return false,
        };
        match outcome {
            EditEntryOutcome::Unhandled => false,
            EditEntryOutcome::Edited => true,
            EditEntryOutcome::Cancel => {
                self.edit_entry = None;
                self.dialogs.close(DialogKind::EditEntry);
                true
            }
            EditEntryOutcome::Invalid(invalid) => {
                self.note(format!("edit entry: {}", invalid.message()));
                true
            }
            EditEntryOutcome::Save(changes) => {
                self.save_edit_entry(changes);
                true
            }
        }
    }

    /// Writes the changed fields into the file's tags and closes the box.
    ///
    /// The write is the honest one
    /// ([`rustamp_meta::write_tags_with_report`]): the fields the user did
    /// not touch keep the value the file has, a field the user **cleared**
    /// is taken out of the file (the removal list of the request names it),
    /// and the report names what the file could not do:
    ///
    /// * an `extra` key the target format refused — a field the format has
    ///   no place for (the total of a disc on an MP3, for example);
    /// * a removal the file still carries, because the value is in a second
    ///   tag of the file (an ID3v1 tag next to an ID3v2 tag) and the writer
    ///   writes only the primary tag.
    ///
    /// Neither is a failure: the rest of the write landed, so the box closes
    /// and the limitation is reported.
    ///
    /// A failed write (an unreadable file, a format that cannot hold the
    /// fields) keeps the box open, so the user can correct the fields and
    /// try again.
    fn save_edit_entry(&mut self, changes: EditEntryChanges) {
        let Some(session) = self.edit_entry.as_ref() else {
            return;
        };
        let path = session.path.clone();
        let report = match rustamp_meta::write_tags_with_report(
            &path,
            &changes.to_metadata(),
            changes.removals(),
        ) {
            Ok(report) => report,
            Err(error) => {
                self.note(format!(
                    "edit entry: {} could not be written: {error}",
                    path.display()
                ));
                return;
            }
        };
        if !report.unwritten_extra.is_empty() {
            self.note(format!(
                "edit entry: {} did not store: {}",
                path.display(),
                report.unwritten_extra.join(", ")
            ));
        }
        if !report.unremoved.is_empty() {
            let fields: Vec<&str> = report
                .unremoved
                .iter()
                .map(|field| tag_field_name(*field))
                .collect();
            self.note(format!(
                "edit entry: {} did not remove: {} (the value is in a second tag of the file)",
                path.display(),
                fields.join(", ")
            ));
        }
        self.edit_entry = None;
        self.dialogs.close(DialogKind::EditEntry);
    }

    /// Confirms the load-playlist dialog: loads the chosen playlist file
    /// through [`App::load_playlist_file`]. Returns whether a path was chosen
    /// and confirmed (the dialog closes on confirm); with nothing chosen the
    /// dialog stays open so the user can retry.
    pub(crate) fn confirm_load_playlist(&mut self) -> bool {
        let Some(path) = self.dialogs.confirm_load_playlist_path() else {
            return false;
        };
        let _ = self.load_playlist_file(&path, true);
        self.dialogs.close_any();
        true
    }

    /// Whether the open dialog has a text field (the presenter sends the OS
    /// text of a key press there instead of the key, see
    /// [`App::dialog_text`]).
    pub fn dialog_accepts_text(&self) -> bool {
        self.edit_entry_session().is_some() || self.dialogs.accepts_text()
    }

    /// Text the OS produced for a key press while a dialog is focused
    /// (layout-aware, with case and punctuation): types it into the open
    /// field, so a full path such as `C:\Music\song.mp3` can be typed (see
    /// [`rustamp_ui::DialogManager::type_text`]). Returns whether anything
    /// was typed; control characters are left to [`App::dialog_key`].
    pub fn dialog_text(&mut self, text: &str) -> bool {
        if self.edit_entry_session().is_some()
            && let Some(session) = self.edit_entry.as_mut()
        {
            return session.dialog.type_text(text);
        }
        let typed = self.dialogs.type_text(text);
        self.sync_jump_rows();
        typed
    }

    /// A file dropped from the shell onto the dialog window: the open-media
    /// and load-playlist dialogs stage it as a pick (`Enter` still confirms).
    /// Returns whether the open dialog took it.
    pub fn dialog_drop_file(&mut self, path: PathBuf) -> bool {
        if self.dialog_is_open(DialogKind::LoadPlaylist) {
            self.dialogs.drop_playlist_path(path)
        } else {
            self.dialogs.drop_media_path(path)
        }
    }

    /// The open-media or load-playlist dialog's entry field text.
    pub fn media_text(&self) -> &str {
        if self.dialog_is_open(DialogKind::LoadPlaylist) {
            return self.dialogs.playlist_text();
        }
        self.dialogs.media_text()
    }

    /// Replaces the open-media or load-playlist dialog's entry field (a full path or URL).
    pub fn set_media_text(&mut self, text: &str) {
        if self.dialog_is_open(DialogKind::LoadPlaylist) {
            self.dialogs.set_playlist_text(text);
            return;
        }
        self.dialogs.set_media_text(text);
    }

    /// Stages the open-media or load-playlist dialog's picks.
    pub fn stage_media_paths(&mut self, paths: Vec<PathBuf>) {
        if self.dialog_is_open(DialogKind::LoadPlaylist) {
            if let Some(first) = paths.into_iter().next() {
                self.dialogs.stage_playlist_path(first);
            }
            return;
        }
        self.dialogs.stage_media_paths(paths);
    }

    /// Stages a playlist file pick for the load-playlist dialog.
    pub fn stage_playlist_path(&mut self, path: PathBuf) {
        self.dialogs.stage_playlist_path(path);
    }

    /// Sets the text in the load-playlist dialog's entry field.
    pub fn set_playlist_text(&mut self, text: &str) {
        self.dialogs.set_playlist_text(text);
    }

    /// The text in the load-playlist dialog's entry field.
    pub fn playlist_text(&self) -> &str {
        self.dialogs.playlist_text()
    }

    /// The jump box's current input text.
    pub fn jump_text(&self) -> &str {
        self.dialogs.jump_text()
    }

    /// The settings the native preferences dialog shows, from live app state.
    pub fn prefs_settings(&self) -> PrefsSettings {
        let entries = self.browser_entries();
        let current = self.current_skin();
        let skin_selected = entries.iter().position(|entry| {
            match (entry.path.as_deref(), current) {
                // The built-in row is the skin in use when no skin was loaded.
                (None, None) => true,
                (Some(path), Some(current)) => crate::skin_folder::same_path(path, current),
                _ => false,
            }
        });
        PrefsSettings {
            always_on_top: self.always_on_top(),
            easy_move: self.easy_move(),
            double_size: self.state().double_size,
            shuffle: self.state().shuffle,
            repeat: self.state().repeat,
            replaygain: self.replaygain_mode(),
            output_mode: self.config().audio.output_mode,
            skins: entries.into_iter().map(|entry| entry.name).collect(),
            skin_selected,
            plugins: self.plugin_rows(),
            library_roots: self.library_roots(),
        }
    }

    /// The rows of every plug-in page's list, from the last scan. Each page
    /// lists its own kind; a page with nothing to list, and any problem with
    /// the folder or the approval store, gets an information row (`id: None`)
    /// instead of an empty list that looks like a working one.
    fn plugin_rows(&self) -> Vec<PluginRow> {
        let scan = self.plugin_scan();
        let config = self.plugins_config();
        let mut rows = Vec::new();
        for node in PLUGIN_NODES {
            let info = |text: String| PluginRow {
                node,
                id: None,
                text,
            };
            if let Some(problem) = &scan.dir_problem {
                rows.push(info(format!("Plug-in folder unreadable: {problem}")));
            }
            if let Some(problem) = &scan.store_problem {
                rows.push(info(format!(
                    "Approval list unusable, nothing is approved: {problem}"
                )));
            }
            let before = rows.len();
            for (index, entry) in scan.entries.iter().enumerate() {
                if plugin_node_for(entry.kind) == node {
                    rows.push(PluginRow {
                        node,
                        id: Some(index),
                        text: entry.row_text(config),
                    });
                }
            }
            if rows.len() == before {
                rows.push(info(format!(
                    "No plug-ins of this type in {}",
                    scan.dir.display()
                )));
            }
        }
        rows
    }

    /// Applies something the user did in the native preferences dialog, through
    /// the same commands the menu and the keys use. A control already at the
    /// requested value changes nothing.
    pub fn apply_prefs_action(&mut self, action: PrefsAction) {
        match action {
            PrefsAction::Set(bind, on) => {
                let current = self.prefs_settings().get(bind);
                if current == on {
                    return;
                }
                let command = match bind {
                    Bind::AlwaysOnTop => Command::ToggleAlwaysOnTop,
                    Bind::EasyMove => Command::ToggleEasyMove,
                    Bind::DoubleSize => Command::ToggleDoubleSize,
                    Bind::Shuffle => Command::SetShuffle(on),
                    Bind::Repeat => Command::SetRepeat(on),
                };
                if let Err(error) = self.submit(command) {
                    self.note(format!("preferences: {error}"));
                }
            }
            PrefsAction::SetReplayGain(mode) => {
                if self.replaygain_mode() == mode {
                    return;
                }
                // The engine runs the mode; the saved configuration takes it
                // from the engine (`App::current_config`).
                self.set_replaygain_mode(mode);
                if let Err(error) = self.save_config() {
                    self.note(format!("preferences: {error}"));
                }
            }
            PrefsAction::SetOutputMode(mode) => {
                if self.config().audio.output_mode == mode {
                    return;
                }
                // The device is opened when the player starts, so the mode is
                // saved now and applied at the next start/device creation.
                self.set_output_mode(mode);
                if let Err(error) = self.save_config() {
                    self.note(format!("preferences: {error}"));
                }
            }
            PrefsAction::LoadSkin(row) => {
                let Some(entry) = self.browser_entries().into_iter().nth(row) else {
                    return;
                };
                match entry.path {
                    Some(path) => {
                        if let Err(error) = self.submit(Command::LoadSkin(path)) {
                            self.note(format!("preferences: {error}"));
                        }
                    }
                    None => self.load_builtin_skin(),
                }
            }
            PrefsAction::TogglePlugin(index) => {
                let Some(entry) = self.plugin_scan().entries.get(index) else {
                    return;
                };
                let on = !entry.is_enabled(self.plugins_config());
                // The request the host gets carries the path and the kind, so
                // they are read before the scan is borrowed again.
                let path = entry.path.clone();
                let kind = entry.kind;
                match self.set_plugin_enabled(index, on) {
                    Ok(()) => {
                        // Saved at once: the choice must not wait for a clean
                        // exit to count.
                        if let Err(error) = self.save_config() {
                            self.note(format!("preferences: {error}"));
                        }
                        if on {
                            // Switching a plug-in on asks the plugin host to
                            // load it: the choice has to reach the host, not
                            // only the configuration file. Only an approved
                            // plug-in gets this far: the approval gate in
                            // `set_plugin_enabled` refuses the rest, so the
                            // host is never asked for an unapproved image.
                            self.request_plugin_load(&path, kind);
                        } else {
                            // The protocol has no unload request, so the
                            // plug-in stays loaded in the host until the host
                            // is replaced. Saying so beats a switch that
                            // looks as if it stopped something.
                            self.note(format!(
                                "the plugin host has no unload request: {} stays loaded until \
                                 the host restarts",
                                path.display()
                            ));
                        }
                    }
                    Err(refused) => self.note(format!("preferences: {refused}")),
                }
            }
            PrefsAction::RescanPlugins => self.rescan_plugins(),
            PrefsAction::AddLibraryRoot => {
                // The dialog does not pick the folder itself: the presenter
                // asks the user for it, from the tick.
                self.library_window.request_folder();
            }
            PrefsAction::RemoveLibraryRoot(row) => {
                // The row is a row of the list the dialog shows; a row that
                // does not exist removes nothing.
                let Some(path) = self.prefs_settings().library_root(row).map(PathBuf::from) else {
                    return;
                };
                if let Err(error) = self.remove_library_root(&path) {
                    self.note(format!("preferences: {error}"));
                }
            }
        }
    }

    /// Renders one frame of `dialog`.
    pub fn render_dialog(&self, dialog: DialogKind) -> Image {
        let body = self.dialog_body(dialog);
        let refs: Vec<&str> = body.iter().map(String::as_str).collect();
        render_dialog(dialog, self.skin.skin(), &refs)
    }

    /// Assembles the body lines for `dialog` from live app state (never
    /// hard-coded track data).
    fn dialog_body(&self, dialog: DialogKind) -> Vec<String> {
        match dialog {
            DialogKind::About => vec![
                format!("{} {}", rustamp_ui::PROGRAM_NAME, env!("CARGO_PKG_VERSION")),
                "A clean-room player.".to_string(),
                "GPL-3.0-or-later.".to_string(),
            ],
            DialogKind::FileInfo => self.file_info_lines(),
            DialogKind::ExtendedInfo => self.extended_info_lines(),
            DialogKind::EditEntry => self.edit_entry_lines(),
            DialogKind::Jump(JumpTarget::Time) => vec![
                "Seconds or m:ss:".to_string(),
                self.dialogs.jump_text().to_string(),
            ],
            DialogKind::Jump(JumpTarget::File) => self.jump_file_lines(),
            // Prompt plus the staged pick or typed field, then the one-line hint.
            DialogKind::LoadPlaylist => self.load_playlist_lines(),
            // Kind-specific faces with plain own-choice prompts (the
            // reference's layout and wording are unmeasured, S04 open): the
            // files/directory faces show the staged picks when the OS
            // picker's stand-in filled them, else the typed field; the
            // location face shows the typed URL.
            DialogKind::OpenMedia { kind, .. } => self.open_media_lines(kind),
            DialogKind::SkinBrowser => self.skin_browser_lines(),
            // Frame only: the reference's page contents are unmeasured (S04
            // open), so the page shows its own-choice name and explicitly states
            // that settings are not applied yet so the UI does not mislead.
            DialogKind::Preferences(page) => vec![
                page_name(page).to_string(),
                "Settings on this page are not applied yet.".to_string(),
            ],
        }
    }

    /// The open-media dialog's body lines for `kind`: the prompt plus the
    /// staged picks (files/directory faces) or the typed field.
    ///
    /// Prompts are plain own choices (the reference's wording is unmeasured,
    /// S04 open); the content lines come from live dialog state, never
    /// hard-coded.
    pub(crate) fn open_media_lines(&self, kind: OpenMediaKind) -> Vec<String> {
        let staged = self.dialogs.staged_media_paths();
        let field = self.dialogs.media_text();
        match kind {
            OpenMediaKind::Files | OpenMediaKind::Directory => {
                let prompt = if kind == OpenMediaKind::Files {
                    "File(s):"
                } else {
                    "Directory:"
                };
                let mut lines = vec![prompt.to_string()];
                if staged.is_empty() {
                    lines.push(if field.is_empty() {
                        "(no file chosen)".to_string()
                    } else {
                        field_tail(field)
                    });
                } else {
                    // Room for three lines between the prompt and the hint.
                    let shown = if staged.len() > 3 { 2 } else { staged.len() };
                    lines.extend(
                        staged[..shown]
                            .iter()
                            .map(|path| field_tail(&path.display().to_string())),
                    );
                    if staged.len() > shown {
                        lines.push(format!("(+{} more)", staged.len() - shown));
                    }
                }
                lines.push(OPEN_MEDIA_HINT.to_string());
                lines
            }
            OpenMediaKind::Location => {
                vec![
                    "Location (URL):".to_string(),
                    field_tail(field),
                    "Type the URL, Enter adds it.".to_string(),
                ]
            }
        }
    }

    /// The load-playlist dialog's body lines: the prompt plus the staged pick
    /// or typed field, then the one-line hint.
    pub(crate) fn load_playlist_lines(&self) -> Vec<String> {
        let field = if let Some(path) = self.dialogs.staged_playlist_path() {
            field_tail(&path.display().to_string())
        } else if self.dialogs.playlist_text().is_empty() {
            String::new()
        } else {
            field_tail(self.dialogs.playlist_text())
        };
        vec![
            "Playlist file:".to_string(),
            field,
            OPEN_PLAYLIST_HINT.to_string(),
        ]
    }

    /// The skin browser's rows from live app state: the built-in default
    /// first, then the skins of the folder beside the configuration, then the
    /// skin in use when it is not one of them.
    ///
    /// The folder is read when the list is asked for and kept until the
    /// folder changes (see [`crate::skin_folder`]), so the list is not read
    /// again for every frame.
    pub(crate) fn browser_entries(&self) -> Vec<SkinEntry> {
        let dir = crate::skin_folder::skins_dir(self.config_path());
        let mut folder = self.skin_folder.borrow_mut();
        let skins = folder.skins(dir.as_deref());
        // The built-in row comes from the UI's own list, so its name lives in
        // one place; the folder's skins follow it.
        let mut entries = skin_entries(None);
        entries.extend(skins.iter().map(|path| SkinEntry {
            // The full path, so two skins with the same file name stay
            // distinct.
            name: path.display().to_string(),
            path: Some(path.clone()),
        }));
        // The skin in use is listed too, unless the folder already lists it.
        if let Some(current) = self.current_skin()
            && !skins
                .iter()
                .any(|skin| crate::skin_folder::same_path(skin, current))
        {
            entries.push(SkinEntry {
                name: current.display().to_string(),
                path: Some(current.clone()),
            });
        }
        entries
    }

    /// The skin browser's body lines: one row per entry, with the selected
    /// row marked. The `"> "` marker uses glyphs the stand-in bitmap font
    /// has; like every dialog line it is own-choice chrome (S04 open).
    pub(crate) fn skin_browser_lines(&self) -> Vec<String> {
        let entries = self.browser_entries();
        let selected = self.dialogs.selected_skin(&entries);
        entries
            .iter()
            .map(|entry| {
                let marker = if Some(entry) == selected { "> " } else { "  " };
                format!("{marker}{}", entry.name)
            })
            .collect()
    }

    /// Confirms the skin browser: loads the selected skin through the
    /// existing skin path and closes the dialog.
    ///
    /// A file row becomes [`Command::LoadSkin`] (with the loader's fallback
    /// and `SkinChanged` event); the built-in row loads the built-in default
    /// skin. A no-op when the browser is not open or has no rows.
    fn confirm_skin_browser(&mut self) {
        let entries = self.browser_entries();
        let Some(selected) = self.dialogs.selected_skin(&entries).cloned() else {
            return;
        };
        match selected.path {
            Some(path) => {
                // A failed submit still consumed the key; the dialog closes
                // either way so a broken pick cannot trap the player.
                let _ = self.submit(Command::LoadSkin(path));
            }
            None => self.load_builtin_skin(),
        }
        self.dialogs.close_any();
    }

    /// The file-info lines for the current track, from the engine snapshot.
    pub(crate) fn file_info_lines(&self) -> Vec<String> {
        let state = self.engine().state();
        let track = state
            .current_index
            .and_then(|index| self.engine().queue().get(index as usize))
            .map(|source| source.display())
            .unwrap_or_else(|| "no track loaded".to_string());
        let stream = match state.stream {
            Some(stream) => {
                let bitrate = stream
                    .bitrate_kbps
                    .map(|kbps| format!("{kbps} kbps, "))
                    .unwrap_or_default();
                format!(
                    "{}{} Hz, {} ch",
                    bitrate, stream.sample_rate_hz, stream.channels
                )
            }
            None => "no stream".to_string(),
        };
        let position = match state.duration_ms {
            Some(duration) => format!("{} / {} ms", state.position_ms, duration),
            None => format!("{} ms", state.position_ms),
        };
        vec![track, stream, position]
    }

    /// Reads the extended info of the selected entries and opens the box
    /// (S03 §2 `40293` "Read extended info on selection").
    ///
    /// The content is an **own choice** (S03 §2 gives the id and the chord
    /// only); the rules are:
    ///
    /// * the selection is the playlist editor's own (S06), in playlist
    ///   order;
    /// * the box names how many entries are selected;
    /// * the tags shown are those of the **first** selected entry — a
    ///   multi-selection is summarised, not read entry by entry, so the
    ///   action stays one file read;
    /// * a URL entry has no local file and therefore no tags: the box says
    ///   so;
    /// * a file that cannot be read is reported the same way — the box
    ///   names the error, and nothing panics.
    ///
    /// With nothing selected the action notes that and opens no box.
    pub(crate) fn show_extended_info(&mut self) {
        let sources = self.selected_sources();
        let Some(first) = sources.first().cloned() else {
            self.note("extended info: nothing is selected".to_string());
            return;
        };
        self.extended_info = Some(extended_info_for(sources.len(), &first));
        self.dialogs.open(DialogKind::ExtendedInfo);
        // The extended-info box replaces the open one, so the edit-entry
        // session (the file it edits) goes with it.
        self.edit_entry = None;
    }

    /// The queue sources of the selected entries, in playlist order.
    ///
    /// The selection is the editor's own (S06): its ids, mapped onto the
    /// engine's queue through the app's id table. An id that is no longer in
    /// the queue is skipped, so a stale selection cannot name a wrong entry.
    fn selected_sources(&self) -> Vec<TrackSource> {
        let mut sources = Vec::new();
        for id in self.playlist.selected_ids() {
            let Some(index) = self.entry_ids.iter().position(|entry| *entry == id) else {
                continue;
            };
            if let Some(source) = self.engine().queue().get(index) {
                sources.push(source.clone());
            }
        }
        sources
    }

    /// The extended-info box's body lines: the snapshot the action took.
    ///
    /// An empty snapshot (the box was opened without a reading) says so
    /// instead of showing nothing.
    pub(crate) fn extended_info_lines(&self) -> Vec<String> {
        match &self.extended_info {
            Some(info) => extended_info_lines(info),
            None => vec!["no extended info was read".to_string()],
        }
    }

    /// Opens the edit-entry box for the selected entry (S03 §2 `40255`
    /// "Edit the playlist entry").
    ///
    /// The box edits the tags of **one** entry: the first selected one, in
    /// playlist order — the same rule the extended-info box uses, so a
    /// multi-selection is summarised instead of read entry by entry.
    ///
    /// The tags are read once, when the action runs, and the box is filled
    /// with them; nothing reads the files again while it is open. The
    /// cases that cannot be edited are reported and open no box:
    ///
    /// * nothing is selected;
    /// * the entry is a stream — it has no local file and therefore no tags
    ///   to edit;
    /// * the file cannot be read.
    pub(crate) fn show_edit_entry(&mut self) {
        let sources = self.selected_sources();
        let Some(first) = sources.first().cloned() else {
            self.note("edit entry: nothing is selected".to_string());
            return;
        };
        let TrackSource::Path(path) = first else {
            self.note(format!(
                "edit entry: {} is a stream and has no tags to edit",
                first.display()
            ));
            return;
        };
        let metadata = match rustamp_meta::read_tags(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.note(format!(
                    "edit entry: {} could not be read: {error}",
                    path.display()
                ));
                return;
            }
        };
        self.edit_entry = Some(EditEntrySession {
            path,
            dialog: EditEntryDialog::new(&metadata),
        });
        self.dialogs.open(DialogKind::EditEntry);
    }

    /// The edit-entry box's body lines: the fields of the entry being edited.
    ///
    /// An empty session (the box was opened without a reading) says so
    /// instead of showing nothing.
    pub(crate) fn edit_entry_lines(&self) -> Vec<String> {
        match &self.edit_entry {
            Some(session) => edit_entry_lines(&session.dialog),
            None => vec!["no entry is being edited".to_string()],
        }
    }
}

/// The extended info of `source`: its tags, or the reason it has none.
///
/// Pure function so the app and its tests share it.
fn extended_info_for(selected: usize, source: &TrackSource) -> ExtendedInfo {
    let mut info = ExtendedInfo {
        selected,
        subject: source.display(),
        ..ExtendedInfo::default()
    };
    match source {
        TrackSource::Url(_) => {
            info.message = Some("a stream has no local file to read tags from".to_string());
        }
        TrackSource::Path(path) => match rustamp_meta::read_tags(path) {
            Ok(metadata) => {
                info.fields = metadata_fields(&metadata);
                if info.fields.is_empty() {
                    info.message = Some("the file has no tags".to_string());
                }
            }
            Err(error) => {
                info.message = Some(format!("the file could not be read: {error}"));
            }
        },
    }
    info
}

/// The tag fields of `metadata`, in a fixed order.
///
/// Own choice: the fields a tagger writes most often, in the order the
/// file-info box names them. A field the file does not carry is left out.
/// The duration is not among them — `rustamp-meta` reads tag containers
/// only and never scans the audio, so the duration stays unknown here (the
/// decoder fills it during playback).
fn metadata_fields(metadata: &TrackMetadata) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    push_field(&mut fields, "Title", metadata.title.clone());
    push_field(&mut fields, "Artist", metadata.artist.clone());
    push_field(&mut fields, "Album", metadata.album.clone());
    push_field(&mut fields, "Album artist", metadata.album_artist.clone());
    push_field(
        &mut fields,
        "Year",
        metadata.year.map(|year| year.to_string()),
    );
    push_field(&mut fields, "Genre", metadata.genre.clone());
    if let Some(track) = metadata.track_no {
        let value = match metadata.track_total {
            Some(total) => format!("{track}/{total}"),
            None => track.to_string(),
        };
        fields.push(("Track".to_string(), value));
    }
    if let Some(disc) = metadata.disc_no {
        fields.push(("Disc".to_string(), disc.to_string()));
    }
    push_field(&mut fields, "Comment", metadata.comment.clone());
    fields
}

/// Adds a field to `fields`, unless the file does not carry it.
fn push_field(fields: &mut Vec<(String, String)>, name: &str, value: Option<String>) {
    if let Some(value) = value {
        fields.push((name.to_string(), value));
    }
}

/// The name of a tag field, as the notes of the app write it.
///
/// The writer names a field it could not remove with the type of the
/// removal ([`rustamp_meta::TagField`]); the note needs a name a reader can
/// make sense of, so the fields are named here.
fn tag_field_name(field: TagField) -> &'static str {
    match field {
        TagField::Title => "title",
        TagField::Artist => "artist",
        TagField::Album => "album",
        TagField::AlbumArtist => "album artist",
        TagField::Year => "year",
        TagField::Genre => "genre",
        TagField::Comment => "comment",
        TagField::Track => "track",
        TagField::Disc => "disc",
    }
}

/// The five plug-in pages of the native preferences dialog.
const PLUGIN_NODES: [PrefsNode; 5] = [
    PrefsNode::Input,
    PrefsNode::Output,
    PrefsNode::PlugInVisualization,
    PrefsNode::DspEffect,
    PrefsNode::GeneralPurpose,
];

/// The plug-in page that lists plug-ins of `kind`.
fn plugin_node_for(kind: PluginKind) -> PrefsNode {
    match kind {
        PluginKind::Input => PrefsNode::Input,
        PluginKind::Output => PrefsNode::Output,
        PluginKind::Visualization => PrefsNode::PlugInVisualization,
        PluginKind::Dsp => PrefsNode::DspEffect,
        PluginKind::General => PrefsNode::GeneralPurpose,
    }
}

/// The page of the native preferences dialog that opens for `page`.
pub fn prefs_node_for(page: PrefsPage) -> PrefsNode {
    match page {
        PrefsPage::VisPlugin => PrefsNode::PlugInVisualization,
        PrefsPage::VisOptions => PrefsNode::OptionsVisualization,
    }
}

/// The open-media file faces' last line: how to reach a file. Own wording
/// (the reference dialog is an OS picker whose labels are unmeasured, S04
/// open); paste is not offered because there is no clipboard binding.
pub(crate) const OPEN_MEDIA_HINT: &str = "Type a path or drop files, Enter adds.";

/// The load-playlist dialog's last line: how to reach a file. Own wording
/// (the reference dialog is an OS picker whose labels are unmeasured, S04 open).
pub(crate) const OPEN_PLAYLIST_HINT: &str = "Type a path or drop a file, Enter loads.";

/// The width of one `text.bmp` cell in pixels. Own choice (the stand-in
/// font's cell, see the playlist rows).
const TEXT_CELL_PX: usize = 5;

/// How many characters of a field fit on one dialog line: the open-media
/// client width less both text pads, in `text.bmp` cells (5 px).
const FIELD_CHARS: usize = (rustamp_ui::OPEN_MEDIA_CLIENT_SIZE.0 as usize
    - 2 * rustamp_ui::dialogs::TEXT_PAD as usize)
    / TEXT_CELL_PX;

/// How many characters of a jump-to-file row fit after its own prefix.
///
/// The row starts with its number (`"> 12. "`), which takes cells of the
/// line, so only the rest is left for the entry's text.
fn jump_row_cells(prefix: usize) -> usize {
    let cells = (rustamp_ui::JUMP_FILE_CLIENT_SIZE.0 as usize
        - 2 * rustamp_ui::dialogs::TEXT_PAD as usize)
        / TEXT_CELL_PX;
    cells.saturating_sub(prefix)
}

/// The visible part of a field: the whole text when it fits in `cells`
/// characters, else `...` plus its tail, so the end of a long path (the part
/// being typed, and the file name) stays on screen.
pub(crate) fn tail_within(text: &str, cells: usize) -> String {
    let count = text.chars().count();
    if count <= cells {
        return text.to_string();
    }
    if cells <= 3 {
        // No room for the ellipsis: the head of the text is all that fits.
        return text.chars().take(cells).collect();
    }
    let keep = cells - 3;
    let tail: String = text.chars().skip(count - keep).collect();
    format!("...{tail}")
}

/// The visible part of a field on an open-media or load-playlist line.
pub(crate) fn field_tail(text: &str) -> String {
    tail_within(text, FIELD_CHARS)
}

/// The dialog manager type, re-exported for the presenter.
pub use rustamp_ui::DialogManager as Dialogs;

#[cfg(test)]
mod tests {
    use rustamp_core::command::{Command, SelectOp};
    use rustamp_core::config::Config;
    use rustamp_core::playlist::{EntryId, MediaLocation};
    use rustamp_ui::ShortcutOutcome;

    use super::*;
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    /// A headless app (no window, no audio device).
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

    /// A copy of the tagged MP3 fixture with an ID3v1 tag behind it.
    ///
    /// The ID3v1 tag is a **second** tag of the file: the writer writes only
    /// the primary tag (the ID3v2 tag here), so a value in the ID3v1 tag
    /// survives a removal.
    fn tagged_mp3_with_id3v1(dir: &tempfile::TempDir, name: &str, title: &str) -> PathBuf {
        let path = tagged_mp3(dir, name);
        let mut bytes = std::fs::read(&path).expect("read the copied fixture");
        // A 128-byte ID3v1 tag: the magic, the title and the genre "none".
        let mut tag = vec![0u8; 128];
        tag[0..3].copy_from_slice(b"TAG");
        let bytes_of_title = title.as_bytes();
        let length = bytes_of_title.len().min(30);
        tag[3..3 + length].copy_from_slice(&bytes_of_title[..length]);
        tag[127] = 255;
        bytes.extend_from_slice(&tag);
        std::fs::write(&path, &bytes).expect("write the file with the ID3v1 tag");
        path
    }

    /// Adds one entry to the queue, selects it and returns its id.
    fn add_and_select(app: &mut App, location: MediaLocation) -> EntryId {
        app.submit(Command::AddItems {
            items: vec![location],
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        let id = *app.entry_ids.last().expect("the entry has an id");
        app.submit(Command::Select(SelectOp::Set(vec![id])))
            .expect("Select applies");
        id
    }

    /// Presses the playlist's `Ctrl+E` row (S03 §2 `40255`).
    fn edit_entry(app: &mut App) -> Option<ShortcutOutcome> {
        app.playlist_key_down(Key::Letter(b'E'), Modifiers::CTRL)
            .expect("Ctrl+E applies")
    }

    /// The open box's body lines.
    fn lines(app: &App) -> Vec<String> {
        app.open_dialog_lines().expect("the box has lines")
    }

    /// The line of the field `label` in the box's body.
    fn field_line(app: &App, label: &str) -> String {
        lines(app)
            .into_iter()
            .find(|line| {
                line.trim_start()
                    .trim_start_matches("> ")
                    .starts_with(label)
            })
            .unwrap_or_else(|| panic!("the box has no {label} line"))
    }

    /// Clears the focused field and types `text` into it.
    fn retype(app: &mut App, text: &str) {
        for _ in 0..64 {
            if !app.dialog_key(Key::Backspace, Modifiers::NONE) {
                break;
            }
        }
        assert!(app.dialog_text(text), "the text was not typed");
    }

    /// Clears the focused field: every character of it goes away.
    fn clear(app: &mut App) {
        for _ in 0..300 {
            if !app.dialog_key(Key::Backspace, Modifiers::NONE) {
                return;
            }
        }
        panic!("the field was not cleared");
    }

    /// Moves the focus to the field `label` with `Tab`.
    fn focus(app: &mut App, label: &str) {
        for _ in 0..16 {
            if field_line(app, label).starts_with("> ") {
                return;
            }
            assert!(
                app.dialog_key(Key::Tab, Modifiers::NONE),
                "Tab was not consumed"
            );
        }
        panic!("the box has no {label} field");
    }

    #[test]
    fn the_two_measured_pages_open_their_native_pages() {
        assert_eq!(
            prefs_node_for(PrefsPage::VisPlugin),
            PrefsNode::PlugInVisualization
        );
        assert_eq!(
            prefs_node_for(PrefsPage::VisOptions),
            PrefsNode::OptionsVisualization
        );
    }

    #[test]
    fn a_long_path_shows_its_tail() {
        let short = r"C:\Music\song.mp3";
        assert_eq!(field_tail(short), short);
        let long = format!(r"C:\{}\song.mp3", "deep\\".repeat(20));
        let shown = field_tail(&long);
        assert!(shown.starts_with("..."), "{shown}");
        assert!(shown.ends_with(r"\song.mp3"), "{shown}");
        assert_eq!(shown.chars().count(), FIELD_CHARS);
    }

    #[test]
    fn a_tail_fits_the_cells_it_is_given() {
        // A text that fits is shown whole.
        assert_eq!(tail_within("abc", 3), "abc");
        assert_eq!(tail_within("abc", 8), "abc");
        // A text that does not shows its tail, with the ellipsis in place of
        // the three cells it takes.
        let shown = tail_within("0123456789", 8);
        assert_eq!(shown, "...56789");
        assert_eq!(shown.chars().count(), 8);
        // Without room for the ellipsis the head of the text is all that
        // fits.
        assert_eq!(tail_within("0123456789", 2), "01");
    }

    #[test]
    fn a_jump_row_leaves_room_for_its_own_number() {
        // The row's prefix takes cells of the line, so the text after it
        // gets only the rest: otherwise the file name would be clipped.
        let cells = jump_row_cells("> 12. ".len());
        let text = "C:\\Music\\a-very-long-file-name-here.mp3";
        let shown = tail_within(text, cells);
        assert!(shown.ends_with(".mp3"), "{shown}");
        let row = format!("> 12. {shown}");
        let budget = (rustamp_ui::JUMP_FILE_CLIENT_SIZE.0 as usize
            - 2 * rustamp_ui::dialogs::TEXT_PAD as usize)
            / TEXT_CELL_PX;
        assert!(row.chars().count() <= budget, "{} > {budget}", row);
    }

    // ----- the edit-entry box (S03 §2 `40255`) -----

    #[test]
    fn the_edit_entry_row_opens_the_box_with_the_tags_of_the_selected_entry() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path));

        let outcome = edit_entry(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(app.dialog_is_open(DialogKind::EditEntry));
        // Every field starts with the value the file carries.
        assert_eq!(field_line(&app, "Title"), "> Title: Sine Title");
        assert_eq!(field_line(&app, "Artist"), "  Artist: Test Artist");
        assert_eq!(field_line(&app, "Album"), "  Album: Test Album");
        assert_eq!(field_line(&app, "Year"), "  Year: 2001");
        assert_eq!(field_line(&app, "Genre"), "  Genre: Test");
        // The last line names the keys, not an error.
        assert!(
            lines(&app).last().expect("a last line").contains("Tab"),
            "{:?}",
            lines(&app)
        );
    }

    #[test]
    fn typing_and_tab_reach_the_fields() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path));
        edit_entry(&mut app);

        // The text of a key press goes into the focused field.
        assert!(app.dialog_text("!"));
        assert_eq!(field_line(&app, "Title"), "> Title: Sine Title!");
        // `Tab` moves the focus to the next field.
        assert!(app.dialog_key(Key::Tab, Modifiers::NONE));
        assert_eq!(field_line(&app, "Artist"), "> Artist: Test Artist");
        assert!(app.dialog_text("?"));
        assert_eq!(field_line(&app, "Artist"), "> Artist: Test Artist?");
        // `Shift`+`Tab` moves it back: the box sees the modifier, so the
        // focus does not run forward.
        assert!(app.dialog_key(Key::Tab, Modifiers::SHIFT));
        assert_eq!(field_line(&app, "Title"), "> Title: Sine Title!");
        assert_eq!(field_line(&app, "Artist"), "  Artist: Test Artist?");
    }

    #[test]
    fn a_bad_year_keeps_the_box_open_with_the_error() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // The year field gets a letter: it is not a number any more.
        focus(&mut app, "Year");
        assert!(app.dialog_text("x"));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        // The box stays open and names the field that does not validate.
        assert!(app.dialog_is_open(DialogKind::EditEntry));
        let last = lines(&app).last().expect("a last line").clone();
        assert!(last.starts_with("Year: "), "{last}");
        assert!(last.contains("must be a number"), "{last}");
        // Nothing was written.
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.year, Some(2001));
    }

    #[test]
    fn save_writes_the_tags_and_reports_the_formats_refusals() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // A new title.
        retype(&mut app, "Edited Title");
        // A disc with a total: the total has no field in an ID3v2 tag, so
        // the format refuses it and the write report says so.
        focus(&mut app, "Disc");
        assert!(app.dialog_text("1/2"));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));

        // The box closed and the file carries the new title.
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.title.as_deref(), Some("Edited Title"));
        // The fields the user did not touch keep the value the file had.
        assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
        assert_eq!(metadata.year, Some(2001));
        // The refusal is reported.
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("DISCTOTAL")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_cleared_field_is_removed_from_the_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // The title is cleared: the value goes away from the file instead of
        // being written as an empty one.
        focus(&mut app, "Title");
        clear(&mut app);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));

        // The box closed and the file lost the title.
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.title, None);
        // The fields the user did not touch keep the value the file had.
        assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
        assert_eq!(metadata.album.as_deref(), Some("Test Album"));
        assert_eq!(metadata.year, Some(2001));
        assert_eq!(metadata.genre.as_deref(), Some("Test"));
        // Nothing was refused, so nothing is reported.
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn a_cleared_field_that_was_empty_removes_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // The comment is empty in the file: there is nothing to take away.
        focus(&mut app, "Comment");
        clear(&mut app);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));

        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.comment, None);
        // The file kept everything it had.
        assert_eq!(metadata.title.as_deref(), Some("Sine Title"));
        assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
        assert_eq!(metadata.album.as_deref(), Some("Test Album"));
        assert_eq!(metadata.year, Some(2001));
        assert_eq!(metadata.track_no, Some(3));
        assert_eq!(metadata.track_total, Some(9));
        // Nothing was refused, so nothing is reported.
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn an_edit_and_a_removal_travel_together() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // A new title ...
        retype(&mut app, "Edited Title");
        // ... and a cleared artist: the one is written, the other goes away.
        focus(&mut app, "Artist");
        clear(&mut app);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));

        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.title.as_deref(), Some("Edited Title"));
        assert_eq!(metadata.artist, None);
        // The fields the user did not touch keep the value the file had.
        assert_eq!(metadata.album.as_deref(), Some("Test Album"));
        assert_eq!(metadata.year, Some(2001));
        assert_eq!(metadata.genre.as_deref(), Some("Test"));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn a_value_in_a_second_tag_is_reported_as_not_removed() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3_with_id3v1(&dir, "tagged.mp3", "V1 Title");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        // The box shows the title of the primary tag.
        assert_eq!(field_line(&app, "Title"), "> Title: Sine Title");
        focus(&mut app, "Title");
        clear(&mut app);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));

        // The write landed and the box closed ...
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        // ... but the ID3v1 tag still has the title, so the file still carries
        // the value and the app says so.
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("did not remove") && note.contains("title")),
            "{notes:?}"
        );
        // The reader merges the two tags: the title comes from the ID3v1 tag.
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.title.as_deref(), Some("V1 Title"));
        // The other fields are untouched.
        assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
    }

    #[test]
    fn a_stream_and_an_empty_selection_report_instead_of_opening() {
        let mut app = test_app();
        // Nothing selected: the action says so and opens no box.
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(PathBuf::from("track.mp3"))],
            at: None,
            play: false,
        })
        .expect("AddItems applies");
        let outcome = edit_entry(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("nothing is selected")),
            "{notes:?}"
        );

        // A stream has no local file and therefore no tags to edit.
        add_and_select(
            &mut app,
            MediaLocation::Url("http://example.com/stream".to_string()),
        );
        let outcome = edit_entry(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("is a stream")),
            "{notes:?}"
        );
    }

    #[test]
    fn escape_closes_the_box_without_writing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = tagged_mp3(&dir, "tagged.mp3");
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(path.clone()));
        edit_entry(&mut app);

        retype(&mut app, "Never Written");
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        // The file kept its title: the box wrote nothing.
        let metadata = rustamp_meta::read_tags(&path).expect("the file is readable");
        assert_eq!(metadata.title.as_deref(), Some("Sine Title"));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_reported_and_opens_no_box() {
        let mut app = test_app();
        add_and_select(&mut app, MediaLocation::File(PathBuf::from("gone.mp3")));
        let outcome = edit_entry(&mut app);
        assert_eq!(outcome, Some(ShortcutOutcome::Handled));
        assert!(!app.dialog_is_open(DialogKind::EditEntry));
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("could not be read")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_box_window_carries_its_own_title_and_size() {
        use rustamp_ui::{EDIT_ENTRY_CLIENT_SIZE, EDIT_ENTRY_TITLE};
        assert_eq!(DialogKind::EditEntry.title(), EDIT_ENTRY_TITLE);
        assert_eq!(DialogKind::EditEntry.client_size(), EDIT_ENTRY_CLIENT_SIZE);
    }
}
