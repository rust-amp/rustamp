// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Simple modal dialogs: About, file info and jump-to (spec S03 actions).
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
//!   choices**, not reference facts. They stay fixed here so tests pin them.
//! * Only one dialog is open at a time: opening a second one replaces the
//!   first. The reference may stack them; that is open.
//! * The jump box takes **digits only** (seconds): the [`Key`] model has no
//!   colon key, so `m:ss` cannot be typed yet. [`parse_jump_time_ms`] still
//!   accepts `m:ss` and `h:mm:ss` for programmatic input and for the day the
//!   key model grows one.
//! * Rendering uses the `text.bmp` bitmap font as a stand-in (the same choice
//!   as the playlist rows; the reference's real dialog font is unknown).

use rustamp_core::command::{Command, JumpTarget};
use rustamp_gfx::{BitmapFont, GlyphMap, Image};
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
}

impl DialogKind {
    /// The window title. Own choice: no spec fixes dialog titles (S04 open).
    pub const fn title(self) -> &'static str {
        match self {
            DialogKind::About => "About Rustamp",
            DialogKind::FileInfo => "File info",
            DialogKind::Jump(JumpTarget::File) => "Jump to file",
            DialogKind::Jump(JumpTarget::Time) => "Jump to time",
        }
    }

    /// The client size in physical pixels. Own choice: no spec fixes dialog
    /// geometry (S04 open).
    pub const fn client_size(self) -> (u32, u32) {
        match self {
            DialogKind::About => ABOUT_CLIENT_SIZE,
            DialogKind::FileInfo => FILE_INFO_CLIENT_SIZE,
            DialogKind::Jump(_) => JUMP_CLIENT_SIZE,
        }
    }
}

/// Client size of the About box. Own choice (see the module docs).
pub const ABOUT_CLIENT_SIZE: (u32, u32) = (260, 110);
/// Client size of the file-info box. Own choice (see the module docs).
pub const FILE_INFO_CLIENT_SIZE: (u32, u32) = (300, 140);
/// Client size of either jump box. Own choice (see the module docs).
pub const JUMP_CLIENT_SIZE: (u32, u32) = (220, 90);

/// Which dialog `command` opens, when it opens one.
///
/// Covers the three dialog commands the shortcuts already emit
/// ([`crate::shortcuts`] maps the S03 keys to them); every other command
/// opens nothing.
pub fn dialog_for_command(command: &Command) -> Option<DialogKind> {
    match command {
        Command::OpenAbout => Some(DialogKind::About),
        Command::OpenFileInfo => Some(DialogKind::FileInfo),
        Command::OpenJump { target } => Some(DialogKind::Jump(*target)),
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

/// The headless dialog state: which dialog (if any) is open, plus the jump
/// box's input text.
#[derive(Debug, Clone, Default)]
pub struct DialogManager {
    open: Option<DialogKind>,
    jump_text: String,
}

impl DialogManager {
    /// No dialog open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `dialog`, replacing whatever is open. Clears the jump input.
    pub fn open(&mut self, dialog: DialogKind) {
        self.open = Some(dialog);
        self.jump_text.clear();
    }

    /// Closes `dialog` when it is the open one; otherwise a no-op.
    pub fn close(&mut self, dialog: DialogKind) {
        if self.open == Some(dialog) {
            self.open = None;
            self.jump_text.clear();
        }
    }

    /// Closes whatever is open, if anything.
    pub fn close_any(&mut self) {
        self.open = None;
        self.jump_text.clear();
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
    /// input (jump dialogs only), `Enter` and `Escape` are consumed by any
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
        ] {
            let command = opening_command(dialog);
            assert_eq!(dialog_for_command(&command), Some(dialog), "{command:?}");
        }
    }

    #[test]
    fn non_dialog_commands_open_nothing() {
        for command in [
            Command::Play,
            Command::Quit,
            Command::OpenPrefs,
            Command::OpenSkinBrowser,
            Command::OpenMainMenu,
            Command::OpenPlaylist,
        ] {
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
        for size in [ABOUT_CLIENT_SIZE, FILE_INFO_CLIENT_SIZE, JUMP_CLIENT_SIZE] {
            assert!(size.0 > 0 && size.1 > 0, "{size:?}");
        }
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
}
