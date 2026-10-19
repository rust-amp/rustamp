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
//! * Jump to time: the prompt plus the typed input. Jump to file opens the
//!   same box shape; confirming it to a playlist index needs the playlist
//!   and stays open (only the time box confirms, to [`Command::Seek`]).
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
//!   plus the current skin when it came from disk), with the selected row
//!   marked. `Enter` loads the selected skin through the existing skin path
//!   (`Command::LoadSkin` for a file row, the built-in loader otherwise) and
//!   closes the dialog.
//! * Preferences: the frame shows the requested page's own-choice name
//!   (`rustamp-prefs`) and nothing else — the reference's page contents are
//!   unmeasured (S04 open), so there are no controls to draw, type into or
//!   confirm. `Enter` stays consumed and keeps the dialog open; `Escape` (or
//!   the window's close button) closes it.

use std::path::PathBuf;

use rustamp_core::command::{Command, JumpTarget, OpenMediaKind, PrefsPage};
use rustamp_gfx::Image;
use rustamp_platform::{Bind, PrefsAction, PrefsNode, PrefsSettings};
use rustamp_prefs::page_name;
use rustamp_ui::{DialogKind, Key, SkinEntry, dialog_for_command, render_dialog, skin_entries};

use crate::app::App;

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
        // The browser moves its selection with the arrow keys, and without the
        // row count a held Down would overrun the end and a following Up would
        // appear dead. Tell the manager how many rows it is looking at.
        if dialog == DialogKind::SkinBrowser {
            self.dialogs.set_skin_rows(self.browser_entries().len());
        }
        Some(dialog)
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
    }

    /// Closes whatever dialog is open, if any. Returns whether one was open.
    pub fn close_open_dialog(&mut self) -> bool {
        let open = self.dialogs.open_kind().is_some();
        self.dialogs.close_any();
        open
    }

    /// A key press while a dialog is focused.
    ///
    /// `Enter` confirms the jump-to-time box (submitting the resulting
    /// [`Command::Seek`]), the skin browser's selected skin, the
    /// load-playlist dialog (loading the file and closing the dialog), or the
    /// open-media dialog (submitting the resulting [`Command::AddItems`] and
    /// closing it; with nothing to confirm the dialog stays open);
    /// `Escape` closes the open dialog; digits type into the jump box (and
    /// letters/digits/spaces into the open-media and load-playlist fields) and
    /// arrows move the skin selection.
    /// Returns `true` when the key was consumed.
    pub fn dialog_key(&mut self, key: Key) -> bool {
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
        match key {
            Key::Enter => {
                if is_load_playlist {
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
        self.dialogs.accepts_text()
    }

    /// Text the OS produced for a key press while a dialog is focused
    /// (layout-aware, with case and punctuation): types it into the open
    /// field, so a full path such as `C:\Music\song.mp3` can be typed (see
    /// [`rustamp_ui::DialogManager::type_text`]). Returns whether anything
    /// was typed; control characters are left to [`App::dialog_key`].
    pub fn dialog_text(&mut self, text: &str) -> bool {
        self.dialogs.type_text(text)
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
        let skin_selected = entries
            .iter()
            .position(|entry| entry.path.as_ref() == current);
        PrefsSettings {
            always_on_top: self.always_on_top(),
            easy_move: self.easy_move(),
            double_size: self.state().double_size,
            shuffle: self.state().shuffle,
            repeat: self.state().repeat,
            skins: entries.into_iter().map(|entry| entry.name).collect(),
            skin_selected,
        }
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
            DialogKind::Jump(JumpTarget::Time) => vec![
                "Seconds or m:ss:".to_string(),
                self.dialogs.jump_text().to_string(),
            ],
            DialogKind::Jump(JumpTarget::File) => vec![
                "Playlist number:".to_string(),
                self.dialogs.jump_text().to_string(),
            ],
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

    /// The skin browser's rows from live app state (see
    /// [`skin_entries`](rustamp_ui::skin_entries)).
    pub(crate) fn browser_entries(&self) -> Vec<SkinEntry> {
        skin_entries(self.current_skin().map(PathBuf::as_path))
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

/// How many characters of a field fit on one dialog line: the open-media
/// client width less both text pads, in `text.bmp` cells (5 px).
const FIELD_CHARS: usize = (rustamp_ui::OPEN_MEDIA_CLIENT_SIZE.0 as usize
    - 2 * rustamp_ui::dialogs::TEXT_PAD as usize)
    / 5;

/// The visible part of a field: the whole text when it fits, else `...` plus
/// its tail, so the end of a long path (the part being typed, and the file
/// name) stays on screen.
pub(crate) fn field_tail(text: &str) -> String {
    let count = text.chars().count();
    if count <= FIELD_CHARS {
        return text.to_string();
    }
    let keep = FIELD_CHARS - 3;
    let tail: String = text.chars().skip(count - keep).collect();
    format!("...{tail}")
}

/// The dialog manager type, re-exported for the presenter.
pub use rustamp_ui::DialogManager as Dialogs;

#[cfg(test)]
mod tests {
    use super::*;

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
}
