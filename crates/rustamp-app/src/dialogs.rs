// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Simple modal dialogs: About, file info, jump-to (S03 actions), the
//! load-playlist dialog (S06 §4 LIST OPTS, RFC 0010) and the skin browser
//! (S03 id 40219, RFC 0005).
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
//! * Load playlist: the spec'd title plus an empty file field. There is no
//!   file picker yet and confirmation is deliberately unmapped: RFC 0010
//!   leaves replace-versus-append (and the file filter) open, so `Enter`
//!   does nothing and the dialog stays open.
//! * Skin browser: the skins the app can load today (the built-in default,
//!   plus the current skin when it came from disk), with the selected row
//!   marked. `Enter` loads the selected skin through the existing skin path
//!   (`Command::LoadSkin` for a file row, the built-in loader otherwise) and
//!   closes the dialog.

use std::path::PathBuf;

use rustamp_core::command::{Command, JumpTarget};
use rustamp_gfx::Image;
use rustamp_ui::{DialogKind, Key, SkinEntry, dialog_for_command, render_dialog, skin_entries};

use crate::app::App;

impl App {
    /// Opens the dialog `command` asks for, when it asks for one the app now
    /// implements (About, file info, either jump box, load playlist, skin
    /// browser).
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

    /// Whether `dialog` is open.
    pub fn dialog_is_open(&self, dialog: DialogKind) -> bool {
        self.dialogs.is_open(dialog)
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
    /// [`Command::Seek`]) or loads the skin browser's selected skin;
    /// `Escape` closes the open dialog; digits type into the jump box and
    /// arrows move the skin selection. `Enter` on the load-playlist dialog is
    /// consumed and does nothing (RFC 0010 leaves its confirmation open).
    /// Returns `true` when the key was consumed.
    pub fn dialog_key(&mut self, key: Key) -> bool {
        let consumed = self.dialogs.key(key);
        if !consumed {
            return false;
        }
        match key {
            Key::Enter => {
                if let Some(seek) = self.dialogs.confirm_jump_time() {
                    // A failed submit still consumed the key: the dialog stays
                    // open so the user can retry.
                    let _ = self.submit(seek);
                } else if self.dialogs.open_kind() == Some(DialogKind::SkinBrowser) {
                    self.confirm_skin_browser();
                }
            }
            Key::Escape => self.dialogs.close_any(),
            _ => {}
        }
        true
    }

    /// The jump box's current input text.
    pub fn jump_text(&self) -> &str {
        self.dialogs.jump_text()
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
            // No file picker yet and no confirmation mapping (RFC 0010
            // leaves the semantics open): the prompt plus an empty field,
            // the same box shape as the jump dialogs.
            DialogKind::LoadPlaylist => vec!["Playlist file:".to_string(), String::new()],
            DialogKind::SkinBrowser => self.skin_browser_lines(),
        }
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

/// The dialog manager type, re-exported for the presenter.
pub use rustamp_ui::DialogManager as Dialogs;
