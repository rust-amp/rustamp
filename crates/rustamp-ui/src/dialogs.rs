// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Simple modal dialogs: About, file info, jump-to (S03 actions), the
//! load-playlist dialog (S06 §4 LIST OPTS, RFC 0010), the skin browser
//! (S03 id 40219, RFC 0005) and the preferences frame (S03 ids 40012, 40191,
//! 40317, RFC 0005 §7).
//!
//! [`DialogKind`] is the headless dialog model: which [`Command`] opens which
//! dialog, each dialog's title and client size, and the open/close state. Like
//! [`crate::behaviour::MainWindow`] it is deliberately **headless**: no
//! `winit`, no clock and no window. The app layer owns one [`DialogManager`]
//! and the OS windows live in `rustamp-app`'s presenter.
//!
//! Deliberate simplifications (stated plainly, because no audited spec covers
//! dialog geometry or contents — S04 is not started):
//!
//! * Titles, client sizes, colours and the exact body lines are **own
//!   choices**, not reference facts — except the load-playlist title, which
//!   S06 §4 settles as `Load Playlist` (shared with the playlist model as
//!   [`crate::playlist::LOAD_DIALOG_TITLE`]). They stay fixed here so tests
//!   pin them.
//! * Only one dialog is open at a time: opening a second one replaces the
//!   first. The reference may stack them; that is open.
//! * The jump box takes **digits only** (seconds): the [`Key`] model has no
//!   colon key, so `m:ss` cannot be typed yet. [`parse_jump_time_ms`] still
//!   accepts `m:ss` and `h:mm:ss` for programmatic input and for the day the
//!   key model grows one.
//! * The load-playlist dialog has **no file picker and no confirmation
//!   command**: RFC 0010 leaves replace-versus-append (and the file filter)
//!   open, so `Enter` is consumed and does nothing. The dialog opens, shows
//!   its spec'd title, and closes.
//! * The skin browser lists exactly the skins the app can load today — the
//!   built-in default plus the current skin when it came from disk — because
//!   the project has no skin search path (no skins-directory concept: the CLI
//!   takes one `--skin` path, the config holds one `ui.skin` path).
//!   Confirming a row loads it through the app's existing skin path.
//! * The preferences dialog is a **frame**: the reference's pages,
//!   controls and layout are unmeasured (S04 open), so it shows the requested
//!   page's name and nothing else. The page list, names and default live in
//!   `rustamp-prefs`; only the two measured pages exist.
//! * Rendering uses the `text.bmp` bitmap font as a stand-in (the same choice
//!   as the playlist rows; the reference's real dialog font is unknown).

use std::path::{Path, PathBuf};

use rustamp_core::command::{Command, JumpTarget, PrefsPage};
use rustamp_gfx::{BitmapFont, GlyphMap, Image};
use rustamp_prefs::DEFAULT_PAGE;
use rustamp_skin::{Sheet, Skin, webamp};

use crate::Key;

/// A dialog the app can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// The About box (S03 "About box", command id 40041).
    About,
    /// The file-info box for the current track (S03 "View file info", id 40188).
    FileInfo,
    /// A jump box (S03 "Jump to file/time", ids 40194/40193).
    Jump(JumpTarget),
    /// The load-playlist dialog (S06 §4 LIST OPTS, RFC 0010
    /// `Command::OpenPlaylist`).
    LoadPlaylist,
    /// The skin browser (S03 "Skin browser", id 40219, RFC 0005
    /// `Command::OpenSkinBrowser`).
    SkinBrowser,
    /// The preferences dialog (S03 "Preferences", id 40012, RFC 0005
    /// `Command::OpenPrefs` / `Command::OpenPrefsPage`).
    ///
    /// Frame only: the reference's pages, controls and layout are unmeasured
    /// (S04 open), so the dialog shows the requested page's name and nothing
    /// else. The payload is the page the command asked for; a bare
    /// `OpenPrefs` shows `rustamp-prefs`' default page, which is an own
    /// choice.
    Preferences(PrefsPage),
}

impl DialogKind {
    /// The window title. Own choice, except the load-playlist title, which
    /// S06 §4 settles (S04 open for the rest).
    pub const fn title(self) -> &'static str {
        match self {
            DialogKind::About => "About Rustamp",
            DialogKind::FileInfo => "File info",
            DialogKind::Jump(JumpTarget::File) => "Jump to file",
            DialogKind::Jump(JumpTarget::Time) => "Jump to time",
            DialogKind::LoadPlaylist => crate::playlist::LOAD_DIALOG_TITLE,
            DialogKind::SkinBrowser => "Skin browser",
            // Own choice, grounded in the measured menu label (S03 id 40012;
            // the menu row reads `Preferences...`, the submenu row
            // `Preferences`): no spec fixes the dialog's title (S04 open).
            DialogKind::Preferences(_) => "Preferences",
        }
    }

    /// The client size in physical pixels. Own choice: no spec fixes dialog
    /// geometry (S04 open).
    pub const fn client_size(self) -> (u32, u32) {
        match self {
            DialogKind::About => ABOUT_CLIENT_SIZE,
            DialogKind::FileInfo => FILE_INFO_CLIENT_SIZE,
            DialogKind::Jump(_) => JUMP_CLIENT_SIZE,
            DialogKind::LoadPlaylist => LOAD_PLAYLIST_CLIENT_SIZE,
            DialogKind::SkinBrowser => SKIN_BROWSER_CLIENT_SIZE,
            DialogKind::Preferences(_) => PREFS_CLIENT_SIZE,
        }
    }
}

/// Client size of the About box. Own choice (see the module docs).
pub const ABOUT_CLIENT_SIZE: (u32, u32) = (260, 110);
/// Client size of the file-info box. Own choice (see the module docs).
pub const FILE_INFO_CLIENT_SIZE: (u32, u32) = (300, 140);
/// Client size of either jump box. Own choice (see the module docs).
pub const JUMP_CLIENT_SIZE: (u32, u32) = (220, 90);
/// Client size of the load-playlist dialog. Own choice (see the module docs).
pub const LOAD_PLAYLIST_CLIENT_SIZE: (u32, u32) = (300, 100);
/// Client size of the skin browser. Own choice: room for the title strip plus
/// the browser rows (built-in plus at most one file row, see
/// [`skin_entries`]).
pub const SKIN_BROWSER_CLIENT_SIZE: (u32, u32) = (300, 140);
/// Client size of the preferences frame. Own choice: room for the title strip
/// plus the page name (see the module docs; no spec fixes dialog geometry,
/// S04 open).
pub const PREFS_CLIENT_SIZE: (u32, u32) = (320, 200);

/// Which dialog `command` opens, when it opens one.
///
/// Covers the dialog commands the shortcuts already emit
/// ([`crate::shortcuts`] maps the S03 keys to them, and the playlist window's
/// LIST OPTS button emits [`Command::OpenPlaylist`] via
/// [`crate::playlist::PlaylistDialog`]); every other command opens nothing.
pub fn dialog_for_command(command: &Command) -> Option<DialogKind> {
    match command {
        Command::OpenAbout => Some(DialogKind::About),
        Command::OpenFileInfo => Some(DialogKind::FileInfo),
        Command::OpenJump { target } => Some(DialogKind::Jump(*target)),
        Command::OpenPlaylist => Some(DialogKind::LoadPlaylist),
        Command::OpenSkinBrowser => Some(DialogKind::SkinBrowser),
        // A bare `OpenPrefs` shows the page model's own-choice default; a
        // page-carrying command shows the page it names (RFC 0005 §7).
        Command::OpenPrefs => Some(DialogKind::Preferences(DEFAULT_PAGE)),
        Command::OpenPrefsPage(page) => Some(DialogKind::Preferences(*page)),
        _ => None,
    }
}

/// The [`Command`] that opens `dialog` (the inverse of
/// [`dialog_for_command`], for tests).
pub fn opening_command(dialog: DialogKind) -> Command {
    match dialog {
        DialogKind::About => Command::OpenAbout,
        DialogKind::FileInfo => Command::OpenFileInfo,
        DialogKind::Jump(target) => Command::OpenJump { target },
        DialogKind::LoadPlaylist => Command::OpenPlaylist,
        DialogKind::SkinBrowser => Command::OpenSkinBrowser,
        // The page-carrying command round-trips; a bare `OpenPrefs` (which
        // opens the default page) has no exact inverse and maps back here.
        DialogKind::Preferences(page) => Command::OpenPrefsPage(page),
    }
}

/// How many jump-input characters are kept (enough for `h:mm:ss`).
pub const MAX_JUMP_LEN: usize = 8;

/// Parses jump-box input into milliseconds.
///
/// Accepts plain seconds (`"90"`), `m:ss` (`"1:30"`) and `h:mm:ss`
/// (`"1:02:03"`); surrounding whitespace is ignored. Anything else is `None`.
/// Pure function so the app and tests share it.
pub fn parse_jump_time_ms(input: &str) -> Option<u64> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut values = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // The first field (hours, minutes or plain seconds) may be any
        // width; every field after it is exactly two digits (`m:ss`,
        // `h:mm:ss`).
        if i > 0 && part.len() != 2 {
            return None;
        }
        let value: u64 = part.parse().ok()?;
        if i > 0 && value >= 60 {
            return None;
        }
        values.push(value);
    }
    let seconds = match values.len() {
        1 => values[0],
        2 => values[0] * 60 + values[1],
        3 => values[0] * 3600 + values[1] * 60 + values[2],
        _ => return None,
    };
    Some(seconds.saturating_mul(1000))
}

/// One row of the skin browser: the skins the app can load today.
///
/// The project has no skin search path (no skins-directory concept: the CLI
/// takes one `--skin` path, the config holds one `ui.skin` path), so the
/// browser lists exactly what the app knows: the built-in default skin and,
/// when the current skin came from disk, that file. `path` is `None` for the
/// built-in row; confirming it loads the built-in default skin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinEntry {
    /// The row text. The built-in row reuses the loader's own description
    /// (`built-in default skin`); a file row is the full path, losslessly, so
    /// two skins with the same file name stay distinct.
    pub name: String,
    /// The skin file or folder, when the row came from disk.
    pub path: Option<PathBuf>,
}

/// Builds the skin browser's rows: the built-in default first, then `current`
/// when it came from disk. Pure function so the app and tests share it.
pub fn skin_entries(current: Option<&Path>) -> Vec<SkinEntry> {
    let mut entries = vec![SkinEntry {
        name: "built-in default skin".to_string(),
        path: None,
    }];
    if let Some(path) = current {
        entries.push(SkinEntry {
            name: path.display().to_string(),
            path: Some(path.to_owned()),
        });
    }
    entries
}

/// The headless dialog state: which dialog (if any) is open, plus the jump
/// box's input text and the skin browser's selected row.
#[derive(Debug, Clone, Default)]
pub struct DialogManager {
    open: Option<DialogKind>,
    jump_text: String,
    skin_index: usize,
    /// How many rows the skin browser has, told to the manager when it opens.
    ///
    /// The manager cannot know the list itself, and without the count a held
    /// Down key would run the index far past the end, so a following Up would
    /// appear to do nothing for several presses. Zero means "not told" and the
    /// moves stay unbounded, which is what the read-side clamp then handles.
    skin_rows: usize,
}

impl DialogManager {
    /// No dialog open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `dialog`, replacing whatever is open. Clears the jump input and
    /// resets the skin selection to the first row.
    pub fn open(&mut self, dialog: DialogKind) {
        self.open = Some(dialog);
        self.jump_text.clear();
        self.skin_index = 0;
    }

    /// Closes `dialog` when it is the open one; otherwise a no-op.
    pub fn close(&mut self, dialog: DialogKind) {
        if self.open == Some(dialog) {
            self.open = None;
            self.jump_text.clear();
            self.skin_index = 0;
        }
    }

    /// Closes whatever is open, if anything.
    pub fn close_any(&mut self) {
        self.open = None;
        self.jump_text.clear();
        self.skin_index = 0;
    }

    /// The open dialog, if any.
    pub fn open_kind(&self) -> Option<DialogKind> {
        self.open
    }

    /// Whether `dialog` is the open one.
    pub fn is_open(&self, dialog: DialogKind) -> bool {
        self.open == Some(dialog)
    }

    /// The jump box's current input text.
    pub fn jump_text(&self) -> &str {
        &self.jump_text
    }

    /// Replaces the jump input (clamped to [`MAX_JUMP_LEN`] characters).
    pub fn set_jump_text(&mut self, text: &str) {
        self.jump_text = text.chars().take(MAX_JUMP_LEN).collect();
    }

    /// Handles one key for the open dialog.
    ///
    /// Returns `true` when the key was consumed: digits append to the jump
    /// input (jump dialogs only), `Up`/`Down` move the skin browser's
    /// selection (skin browser only), `Enter` and `Escape` are consumed by any
    /// open dialog so the app can confirm or close it. Everything else falls
    /// through (`false`).
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Escape | Key::Enter => self.open.is_some(),
            Key::Digit(digit) | Key::Numpad(digit) => {
                if matches!(self.open, Some(DialogKind::Jump(_))) {
                    let digit = digit % 10;
                    if self.jump_text.len() < MAX_JUMP_LEN {
                        self.jump_text.push((b'0' + digit) as char);
                    }
                    true
                } else {
                    false
                }
            }
            Key::Up => {
                if matches!(self.open, Some(DialogKind::SkinBrowser)) {
                    self.select_prev_skin();
                    true
                } else {
                    false
                }
            }
            Key::Down => {
                if matches!(self.open, Some(DialogKind::SkinBrowser)) {
                    self.select_next_skin();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Confirms the jump-to-time box: parses the input and returns the
    /// resulting [`Command::Seek`]. `None` when no jump-to-time dialog is
    /// open or the input does not parse.
    pub fn confirm_jump_time(&self) -> Option<Command> {
        if self.open != Some(DialogKind::Jump(JumpTarget::Time)) {
            return None;
        }
        parse_jump_time_ms(&self.jump_text).map(|to_ms| Command::Seek { to_ms })
    }

    /// The skin browser's selected row index. Unbounded while moving (the
    /// manager never sees the row count); clamped when read through
    /// [`selected_skin`](Self::selected_skin).
    pub fn skin_index(&self) -> usize {
        self.skin_index
    }

    /// Moves the skin browser's selection one row down. A no-op unless the
    /// skin browser is open. Clamped to the last row when the row count is
    /// known (see [`set_skin_rows`](Self::set_skin_rows)).
    pub fn select_next_skin(&mut self) {
        if matches!(self.open, Some(DialogKind::SkinBrowser)) {
            self.skin_index = self.skin_index.saturating_add(1);
            if self.skin_rows > 0 {
                self.skin_index = self.skin_index.min(self.skin_rows - 1);
            }
        }
    }

    /// Tells the manager how many rows the skin browser has, so that moving the
    /// selection saturates at the last row instead of overrunning it. Call it
    /// when opening the browser; `0` means "unknown" and leaves moves unbounded.
    pub fn set_skin_rows(&mut self, rows: usize) {
        self.skin_rows = rows;
        if rows > 0 && self.skin_index >= rows {
            self.skin_index = rows - 1;
        }
    }

    /// Moves the skin browser's selection one row up. A no-op unless the skin
    /// browser is open.
    pub fn select_prev_skin(&mut self) {
        if matches!(self.open, Some(DialogKind::SkinBrowser)) {
            self.skin_index = self.skin_index.saturating_sub(1);
        }
    }

    /// The selected skin-browser row, or `None` when the skin browser is not
    /// open or there are no rows. An overrun index clamps to the last row, so
    /// holding `Down` past the end keeps the last row selected.
    pub fn selected_skin<'a>(&self, entries: &'a [SkinEntry]) -> Option<&'a SkinEntry> {
        if self.open != Some(DialogKind::SkinBrowser) {
            return None;
        }
        entries.get(self.skin_index).or(entries.last())
    }

    /// The preferences dialog's current page, when it is open.
    pub fn prefs_page(&self) -> Option<PrefsPage> {
        match self.open {
            Some(DialogKind::Preferences(page)) => Some(page),
            _ => None,
        }
    }

    /// Switches the open preferences dialog to `page`.
    ///
    /// Returns whether a preferences dialog was open (and now shows `page`).
    /// A page-carrying command for an already-open dialog funnels here through
    /// the app, so menu, shortcut and command paths all switch the same
    /// state; opening replaces the dialog either way (see
    /// [`open`](Self::open)).
    pub fn select_prefs_page(&mut self, page: PrefsPage) -> bool {
        if matches!(self.open, Some(DialogKind::Preferences(_))) {
            self.open = Some(DialogKind::Preferences(page));
            true
        } else {
            false
        }
    }

    /// Confirms the load-playlist dialog: always `None`.
    ///
    /// RFC 0010 leaves replace-versus-append (and the file filter) open, and
    /// no spec fixes the dialog beyond its title, so there is deliberately no
    /// confirmation command yet — confirming must not silently decide
    /// append semantics by emitting `AddItems`. `Enter` stays consumed (see
    /// [`key`](Self::key)) and the dialog stays open.
    pub fn confirm_load_playlist(&self) -> Option<Command> {
        None
    }
}

/// Renders one dialog frame: a filled background, a title strip and the body
/// lines in the `text.bmp` bitmap font.
///
/// `body` carries the dialog's text (the app assembles it: About lines, file
/// lines, the jump prompt plus input). Colours and padding are own choices;
/// a skin without the text sheet still yields the right-sized frame with no
/// text (same fallback shape as the playlist rows).
pub fn render_dialog(kind: DialogKind, skin: &Skin, body: &[&str]) -> Image {
    let (width, height) = kind.client_size();
    let mut canvas = Image::filled(width, height, [0x18, 0x18, 0x20, 0xff]);
    // Title strip: a lighter band with the title text.
    canvas.fill_rect(
        rustamp_gfx::Rect::new(0, 0, width, TITLE_STRIP_HEIGHT),
        [0x3a, 0x3a, 0x4a, 0xff],
    );
    let font = text_font(skin);
    if let Some(font) = font.as_ref() {
        font.render_text(&mut canvas, TEXT_PAD, 3, kind.title());
        let mut y = TITLE_STRIP_HEIGHT as i32 + TEXT_PAD;
        for line in body {
            font.render_text(&mut canvas, TEXT_PAD, y, line);
            y += TEXT_LINE_PX;
        }
    }
    canvas
}

/// Height of the title strip in a dialog frame. Own choice.
pub const TITLE_STRIP_HEIGHT: u32 = 16;
/// Text padding in a dialog frame. Own choice.
pub const TEXT_PAD: i32 = 8;
/// Line advance for dialog body text. Own choice.
pub const TEXT_LINE_PX: i32 = 12;

/// The `text.bmp` bitmap font as a stand-in for the dialogs' real font.
///
/// Mirrors the playlist rows' helper (same glyph map, opaque blits); the
/// reference's real dialog font is unknown (S04 open), so no stand-in can
/// match it.
fn text_font(skin: &Skin) -> Option<BitmapFont> {
    let raster = skin.sheet(Sheet::Text)?;
    let image =
        Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()?;
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    Some(BitmapFont::new(image, glyphs, None, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialog_command_maps_to_its_dialog_and_back() {
        for dialog in [
            DialogKind::About,
            DialogKind::FileInfo,
            DialogKind::Jump(JumpTarget::File),
            DialogKind::Jump(JumpTarget::Time),
            DialogKind::LoadPlaylist,
            DialogKind::SkinBrowser,
            DialogKind::Preferences(PrefsPage::VisPlugin),
            DialogKind::Preferences(PrefsPage::VisOptions),
        ] {
            let command = opening_command(dialog);
            assert_eq!(dialog_for_command(&command), Some(dialog), "{command:?}");
        }
    }

    #[test]
    fn prefs_commands_open_the_prefs_dialog_on_their_page() {
        // A bare `OpenPrefs` shows the page model's own-choice default; a
        // page-carrying command shows the page it names (RFC 0005 §7).
        assert_eq!(
            dialog_for_command(&Command::OpenPrefs),
            Some(DialogKind::Preferences(DEFAULT_PAGE))
        );
        for page in [PrefsPage::VisPlugin, PrefsPage::VisOptions] {
            assert_eq!(
                dialog_for_command(&Command::OpenPrefsPage(page)),
                Some(DialogKind::Preferences(page)),
                "{page:?}"
            );
        }
    }

    #[test]
    fn non_dialog_commands_open_nothing() {
        for command in [Command::Play, Command::Quit, Command::OpenMainMenu] {
            assert_eq!(dialog_for_command(&command), None, "{command:?}");
        }
    }

    #[test]
    fn dialog_sizes_are_fixed_and_non_empty() {
        assert_eq!(DialogKind::About.client_size(), ABOUT_CLIENT_SIZE);
        assert_eq!(DialogKind::FileInfo.client_size(), FILE_INFO_CLIENT_SIZE);
        assert_eq!(
            DialogKind::Jump(JumpTarget::Time).client_size(),
            JUMP_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::LoadPlaylist.client_size(),
            LOAD_PLAYLIST_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::SkinBrowser.client_size(),
            SKIN_BROWSER_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).client_size(),
            PREFS_CLIENT_SIZE
        );
        for size in [
            ABOUT_CLIENT_SIZE,
            FILE_INFO_CLIENT_SIZE,
            JUMP_CLIENT_SIZE,
            LOAD_PLAYLIST_CLIENT_SIZE,
            SKIN_BROWSER_CLIENT_SIZE,
            PREFS_CLIENT_SIZE,
        ] {
            assert!(size.0 > 0 && size.1 > 0, "{size:?}");
        }
    }

    #[test]
    fn the_prefs_title_is_the_measured_menu_label() {
        // Own choice, grounded in the measured menu label (S03 id 40012; no
        // spec fixes the dialog's title, S04 open).
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).title(),
            "Preferences"
        );
    }

    #[test]
    fn the_load_playlist_title_is_the_spec_title() {
        // S06 §4 settles the title from window metadata; it is shared with the
        // playlist model so the two cannot drift.
        assert_eq!(DialogKind::LoadPlaylist.title(), "Load Playlist");
        assert_eq!(
            DialogKind::LoadPlaylist.title(),
            crate::playlist::LOAD_DIALOG_TITLE
        );
    }

    #[test]
    fn opening_replaces_and_closing_clears() {
        let mut manager = DialogManager::new();
        assert_eq!(manager.open_kind(), None);
        manager.open(DialogKind::About);
        assert!(manager.is_open(DialogKind::About));
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert!(!manager.is_open(DialogKind::About));
        assert!(manager.is_open(DialogKind::Jump(JumpTarget::Time)));
        manager.close(DialogKind::About);
        assert!(manager.is_open(DialogKind::Jump(JumpTarget::Time)));
        manager.close_any();
        assert_eq!(manager.open_kind(), None);
    }

    #[test]
    fn jump_time_parses_seconds_and_clock_forms() {
        assert_eq!(parse_jump_time_ms("90"), Some(90_000));
        assert_eq!(parse_jump_time_ms("1:30"), Some(90_000));
        assert_eq!(parse_jump_time_ms("01:30"), Some(90_000));
        assert_eq!(parse_jump_time_ms("1:02:03"), Some(3_723_000));
        assert_eq!(parse_jump_time_ms("  7  "), Some(7_000));
        assert_eq!(parse_jump_time_ms(""), None);
        assert_eq!(parse_jump_time_ms("1:60"), None);
        assert_eq!(parse_jump_time_ms("1:2"), None);
        assert_eq!(parse_jump_time_ms("abc"), None);
        assert_eq!(parse_jump_time_ms("1:2:3:4"), None);
    }

    #[test]
    fn confirming_the_jump_box_seeks_to_the_parsed_time() {
        let mut manager = DialogManager::new();
        assert_eq!(manager.confirm_jump_time(), None);
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert_eq!(manager.confirm_jump_time(), None);
        manager.set_jump_text("1:30");
        assert_eq!(
            manager.confirm_jump_time(),
            Some(Command::Seek { to_ms: 90_000 })
        );
    }

    #[test]
    fn confirming_a_jump_to_file_is_not_a_seek() {
        // Jump-to-file confirms to a playlist index, not a time; that path
        // needs the playlist and stays open, so this box never seeks.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::Jump(JumpTarget::File));
        manager.set_jump_text("90");
        assert_eq!(manager.confirm_jump_time(), None);
    }

    #[test]
    fn dialog_keys_type_digits_and_consume_enter_and_escape() {
        let mut manager = DialogManager::new();
        assert!(!manager.key(Key::Digit(1)));
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert!(manager.key(Key::Digit(9)));
        assert!(manager.key(Key::Numpad(0)));
        assert_eq!(manager.jump_text(), "90");
        assert!(manager.key(Key::Enter));
        assert!(manager.key(Key::Escape));
        assert!(!manager.key(Key::Space));
        // Digits go nowhere on a non-jump dialog.
        manager.open(DialogKind::About);
        assert!(!manager.key(Key::Digit(1)));
        assert!(manager.key(Key::Escape));
    }

    #[test]
    fn the_browser_lists_builtin_first_and_the_current_skin_after_it() {
        let builtin_only = skin_entries(None);
        assert_eq!(builtin_only.len(), 1);
        assert_eq!(builtin_only[0].name, "built-in default skin");
        assert_eq!(builtin_only[0].path, None);

        let current = std::path::Path::new("skins/base.wsz");
        let entries = skin_entries(Some(current));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, None);
        assert_eq!(entries[1].path, Some(current.to_owned()));
        assert_eq!(entries[1].name, current.display().to_string());
    }

    #[test]
    fn the_browser_selection_moves_and_clamps_to_the_last_row() {
        let entries = skin_entries(Some(std::path::Path::new("skins/base.wsz")));
        let mut manager = DialogManager::new();
        // No selection without the browser open.
        assert_eq!(manager.selected_skin(&entries), None);
        manager.open(DialogKind::SkinBrowser);
        manager.set_skin_rows(entries.len());
        assert_eq!(manager.skin_index(), 0);
        assert_eq!(manager.selected_skin(&entries), Some(&entries[0]));
        manager.select_next_skin();
        assert_eq!(manager.selected_skin(&entries), Some(&entries[1]));
        // Holding Down past the end keeps the last row (clamped on read).
        manager.select_next_skin();
        manager.select_next_skin();
        assert_eq!(manager.selected_skin(&entries), Some(&entries[1]));
        manager.select_prev_skin();
        assert_eq!(manager.selected_skin(&entries), Some(&entries[0]));
        // Up at the top stays at the top (saturating).
        manager.select_prev_skin();
        assert_eq!(manager.skin_index(), 0);
        // Reopening resets to the first row.
        manager.select_next_skin();
        manager.open(DialogKind::SkinBrowser);
        manager.set_skin_rows(entries.len());
        assert_eq!(manager.skin_index(), 0);
        // Another dialog open selects nothing.
        manager.open(DialogKind::About);
        assert_eq!(manager.selected_skin(&entries), None);
    }

    #[test]
    fn arrows_move_only_the_skin_browser_and_digits_never_type_there() {
        let mut manager = DialogManager::new();
        assert!(!manager.key(Key::Up));
        assert!(!manager.key(Key::Down));
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert!(!manager.key(Key::Up));
        assert!(!manager.key(Key::Down));
        manager.open(DialogKind::SkinBrowser);
        manager.set_skin_rows(2);
        assert!(manager.key(Key::Down));
        assert_eq!(manager.skin_index(), 1);
        assert!(manager.key(Key::Up));
        assert_eq!(manager.skin_index(), 0);
        assert!(manager.key(Key::Enter));
        assert!(manager.key(Key::Escape));
        // No typing into either new dialog.
        assert!(!manager.key(Key::Digit(1)));
        manager.open(DialogKind::LoadPlaylist);
        assert!(!manager.key(Key::Digit(1)));
        assert!(!manager.key(Key::Up));
        assert!(manager.key(Key::Enter));
        assert!(manager.key(Key::Escape));
    }

    #[test]
    fn confirming_the_load_playlist_dialog_decides_nothing() {
        // RFC 0010 leaves replace-versus-append open, so confirmation must
        // not emit a command (in particular not `AddItems`, which would
        // assert append semantics the spec does not contain).
        let mut manager = DialogManager::new();
        assert_eq!(manager.confirm_load_playlist(), None);
        manager.open(DialogKind::LoadPlaylist);
        assert_eq!(manager.confirm_load_playlist(), None);
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert_eq!(manager.confirm_load_playlist(), None);
    }

    #[test]
    fn the_prefs_page_is_the_open_dialog_and_switches() {
        let mut manager = DialogManager::new();
        assert_eq!(manager.prefs_page(), None);
        // Switching with no preferences dialog open switches nothing.
        assert!(!manager.select_prefs_page(PrefsPage::VisOptions));
        manager.open(DialogKind::Preferences(PrefsPage::VisPlugin));
        assert_eq!(manager.prefs_page(), Some(PrefsPage::VisPlugin));
        assert!(manager.select_prefs_page(PrefsPage::VisOptions));
        assert_eq!(manager.prefs_page(), Some(PrefsPage::VisOptions));
        assert!(manager.is_open(DialogKind::Preferences(PrefsPage::VisOptions)));
        // Another dialog open carries no page and switches nothing.
        manager.open(DialogKind::About);
        assert_eq!(manager.prefs_page(), None);
        assert!(!manager.select_prefs_page(PrefsPage::VisPlugin));
        // Closing clears the page with the dialog.
        manager.open(DialogKind::Preferences(PrefsPage::VisPlugin));
        manager.close_any();
        assert_eq!(manager.prefs_page(), None);
    }

    #[test]
    fn prefs_keys_consume_enter_and_escape_and_type_nothing() {
        // The frame has no controls yet (contents unmeasured), so digits and
        // arrows go nowhere; Enter stays consumed without confirming and
        // Escape closes, like the load-playlist dialog.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::Preferences(PrefsPage::VisPlugin));
        assert!(!manager.key(Key::Digit(1)));
        assert!(!manager.key(Key::Up));
        assert!(!manager.key(Key::Down));
        assert!(manager.key(Key::Enter));
        assert_eq!(manager.prefs_page(), Some(PrefsPage::VisPlugin));
        assert!(manager.key(Key::Escape));
    }
}
