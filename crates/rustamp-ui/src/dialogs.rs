// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Simple modal dialogs: About, file info, jump-to (S03 actions), the
//! load-playlist dialog (S06 §4 LIST OPTS, RFC 0010), the skin browser
//! (S03 id 40219, RFC 0005), the open-media dialogs (S03 ids 40029/40185/40187,
//! RFC 0005 `Command::OpenMedia`) and the preferences frame (S03 ids 40012,
//! 40191, 40317, RFC 0005 §7).
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
//!   [`crate::playlist::LOAD_DIALOG_TITLE`]), and the open-media files title,
//!   which reuses the settled ADD title `Add file(s) to playlist` (shared as
//!   [`crate::playlist::ADD_DIALOG_TITLE`]). They stay fixed here so tests
//!   pin them.
//! * Only one dialog is open at a time: opening a second one replaces the
//!   first. The reference may stack them; that is open.
//! * The jump box takes digits and `:` (seconds, `m:ss` or `h:mm:ss`, see
//!   [`parse_jump_time_ms`]). Digits arrive as [`Key`]s; the colon has no
//!   [`Key`], so it arrives as typed text through
//!   [`DialogManager::type_text`].
//! * The jump-to-file box (S03 id 40194) is a **search box over the
//!   playlist**, not a second time box: it takes any text, lists the entries
//!   whose display text contains it (ignoring case), and moves a selection
//!   with `Up`/`Down` (see [`DialogManager::jump_index`] and
//!   [`DialogManager::set_jump_rows`]). The app confirms the selection to
//!   `Command::PlayEntry`. The reference's list layout, sorting, wording and
//!   result count are unmeasured, so the row shape, the result window and
//!   the count line are own choices.
//! * The load-playlist dialog takes the playlist file the same way the
//!   open-media faces take theirs: typing the full path (any printable
//!   character through [`DialogManager::type_text`]; `Backspace` deletes),
//!   dropping one file onto it, or a staged pick for a later native picker.
//!   Reading the path does not read the file: `Enter` stays consumed and the
//!   dialog stays open (RFC 0010 leaves replace-versus-append open), and the
//!   app half turns the confirmed path into entries plus a skip report
//!   (see [`confirm_load_playlist_path`](DialogManager::confirm_load_playlist_path)
//!   and [`playlist_load_summary_lines`]).
//! * The open-media dialogs have **no OS file picker**: the project may not
//!   add a dependency, and a hand-written native-dialog binding (COM
//!   `IFileOpenDialog` for the directory pick) would be large, unsafe and
//!   untestable headless, while the reference's layout, filters and wording
//!   are unmeasured anyway. Each kind therefore shows a plain entry face — a
//!   file field, a directory field, or a location/URL field — built from this
//!   same machinery. A user reaches a file in one of two ways, both fed by the
//!   OS through the presenter: **typing** the full path (any printable
//!   character, through [`DialogManager::type_text`]; `Backspace` deletes) or
//!   **dropping** files from the shell onto the dialog
//!   ([`DialogManager::drop_media_path`], which stages them as picks).
//!   [`DialogManager::stage_media_paths`] is the same staging for a later
//!   native picker. Confirming produces [`Command::AddItems`] with the opening
//!   command's `play` flag carried through untouched. Clipboard paste is not
//!   supported: `winit` has no clipboard and a Win32 clipboard binding would
//!   be a new dependency. The reference's real layout, filters and wording stay
//!   open (a capture would need the three dialogs' geometry, filter lists and
//!   exact labels).
//! * Typed text keeps its case and punctuation (`C:\Music\song.mp3`); the
//!   older [`Key`] path (letters, digits and spaces) still types upper-case
//!   letters, because the [`Key`] model stores them that way.
//!   [`DialogManager::set_media_text`] replaces the whole field.
//! * The skin browser lists exactly the skins the app can load today — the
//!   built-in default plus the current skin when it came from disk — because
//!   the project has no skin search path (no skins-directory concept: the CLI
//!   takes one `--skin` path, the config holds one `ui.skin` path).
//!   Confirming a row loads it through the app's existing skin path.
//! * The preferences dialog is a **frame**: the reference's pages,
//!   controls and layout are unmeasured (S04 open), so it shows the requested
//!   page's name and nothing else. The page list, names and default live in
//!   `rustamp-prefs`; only the two measured pages exist.
//! * The extended-info box (S03 §2 `40293` "Read extended info on
//!   selection") shows the **tags** of the selected playlist entries: the
//!   count of the selection, the entry the tags belong to (the first
//!   selected one) and its tag fields — or the reason it has none (a stream
//!   has no local file, the file could not be read, it carries no tags).
//!   It is **not** the file-info box, which shows the current track as the
//!   engine sees it. S03 §2 gives the id and the chord only, so the content
//!   is an own choice (see [`ExtendedInfo`]).
//! * The edit-entry box (S03 §2 `40255` "Edit the playlist entry") **edits**
//!   the tags of one playlist entry: a field per tag, pre-filled with the
//!   value the file has, with its own focus, validation and a save request
//!   that carries the changed fields only (see [`EditEntryDialog`]). The box
//!   writes nothing itself: the app reads the tags, opens the box with them
//!   and hands the request to the tag writer. S03 §2 gives the id and the
//!   chord only, so the fields, the layout and the validation are own
//!   choices.
//! * Rendering uses the `text.bmp` bitmap font as a stand-in (the same choice
//!   as the playlist rows; the reference's real dialog font is unknown).

use std::path::{Path, PathBuf};

use rustamp_core::command::{Command, JumpTarget, OpenMediaKind, PrefsPage};
use rustamp_core::meta::TrackMetadata;
use rustamp_gfx::{BitmapFont, GlyphMap, Image};
use rustamp_prefs::DEFAULT_PAGE;
use rustamp_skin::{Sheet, Skin, webamp};

use crate::{Key, Modifiers};

/// A dialog the app can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// The About box (S03 "About box", command id 40041).
    About,
    /// The file-info box for the current track (S03 "View file info", id 40188).
    FileInfo,
    /// The extended-info box for the playlist selection (S03 §2 "Read extended
    /// info on selection", id 40293).
    ///
    /// **Not** the file-info box: that one shows the *current track* as the
    /// engine sees it (bitrate, sample rate, channels), while this one shows
    /// the *tags* of the selected playlist entries. The two answer different
    /// questions, so they are two dialogs.
    ///
    /// No core command opens it (reading a file's tags is not a player
    /// action, RFC 0005 §9), so the app opens it itself; the content is
    /// [`ExtendedInfo`].
    ExtendedInfo,
    /// The edit-entry box (S03 §2 `40255` "Edit the playlist entry").
    ///
    /// It **edits** the tags of one playlist entry: the app reads the tags
    /// of the selected entry, opens the box with them and hands the save
    /// request to the tag writer. No core command opens it (editing a file's
    /// tags is not a player action, RFC 0005 §9), so the app opens it
    /// itself; the content is [`EditEntryDialog`].
    EditEntry,
    /// A jump box (S03 "Jump to file/time", ids 40194/40193).
    Jump(JumpTarget),
    /// The load-playlist dialog (S06 §4 LIST OPTS, RFC 0010
    /// `Command::OpenPlaylist`).
    LoadPlaylist,
    /// An open-media dialog (RFC 0005 `Command::OpenMedia`): a file picker
    /// face, a directory picker face, or a location/URL entry field, per
    /// `kind`.
    ///
    /// `play` is carried through from the opening command untouched (it
    /// decides whether confirming starts the first added entry); the dialog
    /// itself never invents it. The playlist ADD path opens
    /// `{ kind: Files, play: false }` (RFC 0005 O5: the playlist add's `play`
    /// value is unmeasured), the main-window actions open their kind with
    /// `play: true`.
    OpenMedia { kind: OpenMediaKind, play: bool },
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
            // Own choice: S03 §2 gives the row's id and chord, not a title,
            // and no spec fixes dialog titles (S04 open).
            DialogKind::ExtendedInfo => "Extended info",
            // Own choice: S03 §2 gives the row's id and chord, not a title,
            // and no spec fixes dialog titles (S04 open).
            DialogKind::EditEntry => EDIT_ENTRY_TITLE,
            DialogKind::Jump(JumpTarget::File) => "Jump to file",
            DialogKind::Jump(JumpTarget::Time) => "Jump to time",
            DialogKind::LoadPlaylist => crate::playlist::LOAD_DIALOG_TITLE,
            // The files face reuses the settled ADD title (S06 §4, shared so
            // the two cannot drift); the directory/location titles are own
            // choices — no spec fixes them (S04 open).
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                ..
            } => crate::playlist::ADD_DIALOG_TITLE,
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                ..
            } => "Open directory",
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Location,
                ..
            } => "Open location",
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
            DialogKind::ExtendedInfo => EXTENDED_INFO_CLIENT_SIZE,
            DialogKind::EditEntry => EDIT_ENTRY_CLIENT_SIZE,
            DialogKind::Jump(JumpTarget::Time) => JUMP_CLIENT_SIZE,
            DialogKind::Jump(JumpTarget::File) => JUMP_FILE_CLIENT_SIZE,
            DialogKind::LoadPlaylist => LOAD_PLAYLIST_CLIENT_SIZE,
            DialogKind::OpenMedia { .. } => OPEN_MEDIA_CLIENT_SIZE,
            DialogKind::SkinBrowser => SKIN_BROWSER_CLIENT_SIZE,
            DialogKind::Preferences(_) => PREFS_CLIENT_SIZE,
        }
    }
}

/// Client size of the About box. Own choice (see the module docs).
pub const ABOUT_CLIENT_SIZE: (u32, u32) = (260, 110);
/// Client size of the file-info box. Own choice (see the module docs).
pub const FILE_INFO_CLIENT_SIZE: (u32, u32) = (300, 140);
/// Client size of the extended-info box: a count line, the entry the tags
/// belong to and the tag fields. Own choice (see the module docs).
pub const EXTENDED_INFO_CLIENT_SIZE: (u32, u32) = (320, 190);
/// Client size of the jump-to-time box. Own choice (see the module docs).
pub const JUMP_CLIENT_SIZE: (u32, u32) = (220, 90);
/// Client size of the jump-to-file box: a search field plus
/// [`JUMP_FILE_ROWS`] result rows. Own choice (see the module docs).
pub const JUMP_FILE_CLIENT_SIZE: (u32, u32) = (300, 190);
/// How many result rows the jump-to-file box shows at once.
pub const JUMP_FILE_ROWS: usize = 10;
/// The longest search text the jump-to-file box keeps.
pub const MAX_JUMP_FILE_LEN: usize = 40;
/// Client size of the load-playlist dialog. Own choice (see the module docs).
pub const LOAD_PLAYLIST_CLIENT_SIZE: (u32, u32) = (300, 100);
/// Client size of each open-media dialog. Own choice: the same box shape as
/// the load-playlist dialog (no spec fixes dialog geometry, S04 open).
pub const OPEN_MEDIA_CLIENT_SIZE: (u32, u32) = (300, 100);
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
/// ([`crate::shortcuts`] maps the S03 keys to them, the playlist window's
/// ADD button emits [`Command::OpenMedia`] and its LIST OPTS button emits
/// [`Command::OpenPlaylist`] via
/// [`crate::playlist::PlaylistDialog`]); every other command opens nothing.
pub fn dialog_for_command(command: &Command) -> Option<DialogKind> {
    match command {
        Command::OpenAbout => Some(DialogKind::About),
        Command::OpenFileInfo => Some(DialogKind::FileInfo),
        Command::OpenJump { target } => Some(DialogKind::Jump(*target)),
        Command::OpenPlaylist => Some(DialogKind::LoadPlaylist),
        // The `play` flag rides along untouched: confirming the dialog emits
        // `AddItems` with this same value (RFC 0005 §5.2; the `false` of the
        // playlist ADD path is unmeasured, O5, and stays as the caller sent
        // it).
        Command::OpenMedia { kind, play } => Some(DialogKind::OpenMedia {
            kind: *kind,
            play: *play,
        }),
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
///
/// `None` for a dialog no command opens: the extended-info box is opened
/// by the playlist's `Ctrl+Alt+E` action (S03 §2 `40293`) and the
/// edit-entry box by its `Ctrl+E` action (S03 §2 `40255`), because reading
/// or editing a file's tags is not a player action and needs no core
/// command (RFC 0005 §9).
pub fn opening_command(dialog: DialogKind) -> Option<Command> {
    match dialog {
        DialogKind::About => Some(Command::OpenAbout),
        DialogKind::FileInfo => Some(Command::OpenFileInfo),
        DialogKind::Jump(target) => Some(Command::OpenJump { target }),
        DialogKind::LoadPlaylist => Some(Command::OpenPlaylist),
        DialogKind::OpenMedia { kind, play } => Some(Command::OpenMedia { kind, play }),
        DialogKind::SkinBrowser => Some(Command::OpenSkinBrowser),
        // The page-carrying command round-trips; a bare `OpenPrefs` (which
        // opens the default page) has no exact inverse and maps back here.
        DialogKind::Preferences(page) => Some(Command::OpenPrefsPage(page)),
        // No command opens the extended-info box or the edit-entry box; the
        // app opens them itself.
        DialogKind::ExtendedInfo | DialogKind::EditEntry => None,
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

/// How many entry-field characters an open-media dialog keeps.
///
/// Own choice: long enough for a deep path or a URL with a query string; the
/// reference's limits (if any) are unmeasured.
pub const MAX_MEDIA_LEN: usize = 512;

/// Maps staged file/directory picks to playlist locations.
///
/// One pick becomes one [`MediaLocation::File`], in order. A directory pick
/// stays a plain file path: no enumerated expansion exists yet (the app adds
/// the path as-is), which stays open with the reference's directory
/// semantics. Pure function so the app and tests share it.
pub fn media_items_for_paths(paths: &[PathBuf]) -> Vec<rustamp_core::playlist::MediaLocation> {
    paths
        .iter()
        .map(|path| rustamp_core::playlist::MediaLocation::File(path.clone()))
        .collect()
}

/// Maps a location-dialog entry to its playlist location.
///
/// The trimmed text becomes [`MediaLocation::Url`]; blank input is `None`
/// (nothing to confirm). No validation beyond that: URL syntax is unmeasured,
/// so anything non-blank is passed through. Pure function so the app and
/// tests share it.
pub fn media_item_for_url(text: &str) -> Option<rustamp_core::playlist::MediaLocation> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(rustamp_core::playlist::MediaLocation::Url(
        trimmed.to_string(),
    ))
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
/// box's input text, the skin browser's selected row and the open-media
/// dialog's entry field and staged picks.
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
    /// The jump-to-file box's selected result row, and how many rows the app
    /// last reported (the same bounded-move scheme as the skin browser).
    jump_index: usize,
    jump_rows: usize,
    /// The open-media dialog's entry field (typed path or URL text).
    media_text: String,
    /// The open-media dialog's staged picks (the stand-in for the OS picker's
    /// result; see the module docs).
    media_paths: Vec<PathBuf>,
    /// The load-playlist dialog's entry field (a typed playlist path).
    playlist_text: String,
    /// The load-playlist dialog's staged pick (the stand-in for the OS
    /// picker's result; a single playlist file).
    playlist_paths: Vec<PathBuf>,
}

impl DialogManager {
    /// No dialog open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `dialog`, replacing whatever is open. Clears the jump input, the
    /// open-media entry field and its staged picks, the load-playlist field
    /// and its staged pick, and resets the skin
    /// selection to the first row.
    pub fn open(&mut self, dialog: DialogKind) {
        self.open = Some(dialog);
        self.jump_text.clear();
        self.jump_index = 0;
        self.skin_index = 0;
        self.media_text.clear();
        self.media_paths.clear();
        self.playlist_text.clear();
        self.playlist_paths.clear();
    }

    /// Closes `dialog` when it is the open one; otherwise a no-op.
    pub fn close(&mut self, dialog: DialogKind) {
        if self.open == Some(dialog) {
            self.open = None;
            self.jump_text.clear();
            self.jump_index = 0;
            self.skin_index = 0;
            self.media_text.clear();
            self.media_paths.clear();
            self.playlist_text.clear();
            self.playlist_paths.clear();
        }
    }

    /// Closes whatever is open, if anything.
    pub fn close_any(&mut self) {
        self.open = None;
        self.jump_text.clear();
        self.jump_index = 0;
        self.skin_index = 0;
        self.media_text.clear();
        self.media_paths.clear();
        self.playlist_text.clear();
        self.playlist_paths.clear();
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

    /// Replaces the jump input (clamped to [`MAX_JUMP_LEN`] characters, or
    /// [`MAX_JUMP_FILE_LEN`] for the jump-to-file search).
    pub fn set_jump_text(&mut self, text: &str) {
        self.jump_text = text.chars().take(self.jump_limit()).collect();
        self.jump_index = 0;
    }

    fn jump_limit(&self) -> usize {
        if self.open == Some(DialogKind::Jump(JumpTarget::File)) {
            MAX_JUMP_FILE_LEN
        } else {
            MAX_JUMP_LEN
        }
    }

    /// Appends one character to the jump input (a no-op at the limit) and
    /// puts the result selection back on the first row, since the search
    /// text changed.
    fn push_jump_char(&mut self, ch: char) {
        if self.jump_text.chars().count() < self.jump_limit() {
            self.jump_text.push(ch);
            self.jump_index = 0;
        }
    }

    /// The jump-to-file box's selected result row, clamped to the rows the
    /// app last reported (see [`set_jump_rows`](Self::set_jump_rows)).
    pub fn jump_index(&self) -> usize {
        match self.jump_rows {
            0 => 0,
            rows => self.jump_index.min(rows - 1),
        }
    }

    /// Tells the manager how many results the jump-to-file box has now, so
    /// the selection stays inside them. The app calls it after every change
    /// of the search text.
    ///
    /// An empty result list puts the selection back on the first row: there
    /// is no row to point at, and a stale index would select an unrelated
    /// entry as soon as the next search matches again.
    pub fn set_jump_rows(&mut self, rows: usize) {
        self.jump_rows = rows;
        if rows > 0 {
            self.jump_index = self.jump_index.min(rows - 1);
        } else {
            self.jump_index = 0;
        }
    }

    /// Handles one key for the open dialog.
    ///
    /// Returns `true` when the key was consumed: digits append to the jump
    /// input (jump dialogs only), letters/digits/spaces append to the
    /// open-media entry field (open-media dialogs only; the [`Key`] model has
    /// no punctuation, so full paths and URLs arrive through
    /// [`set_media_text`](Self::set_media_text)), `Up`/`Down` move the skin
    /// browser's selection (skin browser only), `Backspace` deletes the last
    /// character of the jump input or the open-media field, `Enter` and
    /// `Escape` are consumed by any open dialog so the app can confirm or
    /// close it. Everything else falls through (`false`).
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Escape | Key::Enter => self.open.is_some(),
            Key::Backspace => match self.open {
                Some(DialogKind::Jump(_)) => {
                    self.jump_text.pop();
                    self.jump_index = 0;
                    true
                }
                Some(DialogKind::OpenMedia { .. }) => {
                    self.media_text.pop();
                    true
                }
                Some(DialogKind::LoadPlaylist) => {
                    self.playlist_text.pop();
                    true
                }
                _ => false,
            },
            Key::Digit(digit) | Key::Numpad(digit) => {
                if matches!(self.open, Some(DialogKind::Jump(_))) {
                    let digit = digit % 10;
                    self.push_jump_char((b'0' + digit) as char);
                    true
                } else if matches!(self.open, Some(DialogKind::OpenMedia { .. })) {
                    let digit = digit % 10;
                    self.push_media_char((b'0' + digit) as char);
                    true
                } else {
                    false
                }
            }
            Key::Letter(letter) => {
                // The key model stores letters upper-case, so typed text keeps
                // that case; exact text arrives through `set_media_text`.
                if matches!(self.open, Some(DialogKind::OpenMedia { .. })) {
                    self.push_media_char(letter as char);
                    true
                } else if self.open == Some(DialogKind::Jump(JumpTarget::File)) {
                    self.push_jump_char(letter as char);
                    true
                } else {
                    false
                }
            }
            Key::Space => {
                if matches!(self.open, Some(DialogKind::OpenMedia { .. })) {
                    self.push_media_char(' ');
                    true
                } else if self.open == Some(DialogKind::Jump(JumpTarget::File)) {
                    self.push_jump_char(' ');
                    true
                } else {
                    false
                }
            }
            Key::Up => match self.open {
                Some(DialogKind::SkinBrowser) => {
                    self.select_prev_skin();
                    true
                }
                Some(DialogKind::Jump(JumpTarget::File)) => {
                    self.jump_index = self.jump_index.saturating_sub(1);
                    true
                }
                _ => false,
            },
            Key::Down => match self.open {
                Some(DialogKind::SkinBrowser) => {
                    self.select_next_skin();
                    true
                }
                Some(DialogKind::Jump(JumpTarget::File)) => {
                    self.jump_index = self.jump_index.saturating_add(1);
                    if self.jump_rows > 0 {
                        self.jump_index = self.jump_index.min(self.jump_rows - 1);
                    }
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether the open dialog has a text field [`type_text`](Self::type_text)
    /// can type into (the jump boxes, the open-media faces and the
    /// load-playlist dialog).
    pub fn accepts_text(&self) -> bool {
        matches!(
            self.open,
            Some(DialogKind::Jump(_))
                | Some(DialogKind::OpenMedia { .. })
                | Some(DialogKind::LoadPlaylist)
        )
    }

    /// Types `text` into the open dialog's field, as the OS delivered it
    /// (layout-aware, with case and punctuation).
    ///
    /// Control characters (`Enter`'s `\r`, `Backspace`'s `\u{8}`, `Escape`,
    /// `Ctrl+letter` codes) are dropped: those keys take the [`key`](Self::key)
    /// path. The open-media field takes every other character, so a full path
    /// such as `C:\Music\song.mp3` or a URL can be typed; the load-playlist
    /// field takes every other character the same way, so a playlist path can
    /// be typed; the jump box takes digits and `:` (its [`parse_jump_time_ms`]
    /// forms). Returns `true` when at least one character was typed into a
    /// field.
    pub fn type_text(&mut self, text: &str) -> bool {
        let mut typed = false;
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            match self.open {
                Some(DialogKind::OpenMedia { .. }) => {
                    self.push_media_char(ch);
                    typed = true;
                }
                Some(DialogKind::LoadPlaylist) => {
                    if self.playlist_text.chars().count() < MAX_MEDIA_LEN {
                        self.playlist_text.push(ch);
                    }
                    typed = true;
                }
                Some(DialogKind::Jump(JumpTarget::Time)) if ch.is_ascii_digit() || ch == ':' => {
                    self.push_jump_char(ch);
                    typed = true;
                }
                Some(DialogKind::Jump(JumpTarget::File)) => {
                    self.push_jump_char(ch);
                    typed = true;
                }
                _ => {}
            }
        }
        typed
    }

    /// Adds a file the user dropped onto the open-media dialog from the shell.
    ///
    /// The files face collects every drop as a staged pick (one drop event
    /// per file), the directory face keeps the last drop as its single pick;
    /// the location face takes no files. Returns whether the drop was taken.
    /// Confirming still needs `Enter`, so a stray drop adds nothing.
    pub fn drop_media_path(&mut self, path: PathBuf) -> bool {
        match self.open {
            Some(DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                ..
            }) => {
                self.media_paths.push(path);
                true
            }
            Some(DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                ..
            }) => {
                self.media_paths = vec![path];
                true
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

    /// The open-media dialog's entry field text.
    pub fn media_text(&self) -> &str {
        &self.media_text
    }

    /// Replaces the open-media entry field (clamped to [`MAX_MEDIA_LEN`]
    /// characters).
    ///
    /// Typing goes through [`type_text`](Self::type_text) instead; this
    /// replaces the whole field at once (embedding and tests). Dropped files
    /// and a later native picker fill the staged picks; all of them feed
    /// [`confirm_open_media`](Self::confirm_open_media).
    pub fn set_media_text(&mut self, text: &str) {
        self.media_text = text.chars().take(MAX_MEDIA_LEN).collect();
    }

    /// Stages open-media picks (the stand-in for the OS picker's result).
    ///
    /// A later native file/directory picker fills this instead of the entry
    /// field; [`confirm_open_media`](Self::confirm_open_media) reads it first.
    pub fn stage_media_paths(&mut self, paths: Vec<PathBuf>) {
        self.media_paths = paths;
    }

    /// The staged open-media picks, if any.
    pub fn staged_media_paths(&self) -> &[PathBuf] {
        &self.media_paths
    }

    /// The load-playlist dialog's entry field text (a typed playlist path).
    pub fn playlist_text(&self) -> &str {
        &self.playlist_text
    }

    /// Replaces the load-playlist entry field (clamped to [`MAX_MEDIA_LEN`]
    /// characters).
    ///
    /// Typing goes through [`type_text`](Self::type_text) instead; this
    /// replaces the whole field at once (embedding and tests).
    pub fn set_playlist_text(&mut self, text: &str) {
        self.playlist_text = text.chars().take(MAX_MEDIA_LEN).collect();
    }

    /// Stages the load-playlist pick (the stand-in for the OS picker's
    /// result: a single playlist file).
    ///
    /// [`confirm_load_playlist_path`](Self::confirm_load_playlist_path)
    /// reads it first.
    pub fn stage_playlist_path(&mut self, path: PathBuf) {
        self.playlist_paths = vec![path];
    }

    /// The staged load-playlist pick, if any.
    pub fn staged_playlist_path(&self) -> Option<&PathBuf> {
        self.playlist_paths.first()
    }

    /// Adds a file the user dropped onto the load-playlist dialog from the
    /// shell (one drop event per file; the last drop wins — a single playlist
    /// file). Returns whether the drop was taken.
    pub fn drop_playlist_path(&mut self, path: PathBuf) -> bool {
        if self.open != Some(DialogKind::LoadPlaylist) {
            return false;
        }
        self.playlist_paths = vec![path];
        true
    }

    /// Confirms the load-playlist dialog: the playlist file to read.
    ///
    /// The staged pick wins; otherwise the trimmed entry field is one path.
    /// `None` when no load-playlist dialog is open or there is nothing to
    /// confirm. Reading the file, appending versus replacing, and reporting
    /// skips stay with the app (RFC 0010): this hands over the path only.
    pub fn confirm_load_playlist_path(&self) -> Option<PathBuf> {
        if self.open != Some(DialogKind::LoadPlaylist) {
            return None;
        }
        if let Some(first) = self.playlist_paths.first() {
            return Some(first.clone());
        }
        let trimmed = self.playlist_text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(PathBuf::from(trimmed))
    }

    /// Appends one typed character to the open-media entry field, clamped to
    /// [`MAX_MEDIA_LEN`] characters.
    fn push_media_char(&mut self, ch: char) {
        if self.media_text.chars().count() < MAX_MEDIA_LEN {
            self.media_text.push(ch);
        }
    }

    /// Confirms the open-media dialog: turns the staged picks or the entry
    /// field into the resulting [`Command::AddItems`].
    ///
    /// * `Location` reads the entry field: trimmed non-blank text becomes one
    ///   [`MediaLocation::Url`](rustamp_core::playlist::MediaLocation::Url).
    /// * `Files` reads the staged picks in order (each a
    ///   [`MediaLocation::File`](rustamp_core::playlist::MediaLocation::File)),
    ///   falling back to the entry field as a single file path.
    /// * `Directory` reads the first staged pick (or the entry field), as a
    ///   single file path — no directory expansion exists yet (see
    ///   [`media_items_for_paths`]).
    ///
    /// The command appends (`at: None`) and carries the opening command's
    /// `play` flag untouched — in particular the playlist ADD path
    /// (`play: false`, RFC 0005 O5) stays `false` here rather than being
    /// decided by the dialog. `None` when no open-media dialog is open or
    /// there is nothing to confirm (the dialog stays open so the user can
    /// retry).
    pub fn confirm_open_media(&self) -> Option<Command> {
        let (kind, play) = match self.open {
            Some(DialogKind::OpenMedia { kind, play }) => (kind, play),
            _ => return None,
        };
        let items = match kind {
            OpenMediaKind::Location => vec![media_item_for_url(&self.media_text)?],
            OpenMediaKind::Files => {
                if self.media_paths.is_empty() {
                    let trimmed = self.media_text.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    vec![rustamp_core::playlist::MediaLocation::File(PathBuf::from(
                        trimmed,
                    ))]
                } else {
                    media_items_for_paths(&self.media_paths)
                }
            }
            OpenMediaKind::Directory => {
                if let Some(first) = self.media_paths.first() {
                    vec![rustamp_core::playlist::MediaLocation::File(first.clone())]
                } else {
                    let trimmed = self.media_text.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    vec![rustamp_core::playlist::MediaLocation::File(PathBuf::from(
                        trimmed,
                    ))]
                }
            }
        };
        Some(Command::AddItems {
            items,
            at: None,
            play,
        })
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
}

/// The content of the extended-info box (S03 §2 `40293`).
///
/// A **snapshot**: the app fills it when the action runs and the box shows
/// it. The app owns the reading of the tags (`rustamp-meta` lives there);
/// this type only carries what was read, so the box stays headless.
///
/// S03 §2 gives the command id and the chord only, so the content is an
/// **own choice**. The rules it follows:
///
/// * `selected` is how many entries are selected;
/// * `subject` is the entry the fields belong to — the **first** selected
///   one, in playlist order. A multi-selection is summarised, not read
///   entry by entry, so the action stays one file read;
/// * `fields` are the tag fields of `subject`, as `(name, value)` pairs in
///   the order the app read them;
/// * `message` says why there are no fields: a stream has no local file,
///   the file could not be read, or it carries no tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedInfo {
    /// How many entries are selected.
    pub selected: usize,
    /// The entry the fields belong to: the first selected one, in playlist
    /// order. Its path or URL.
    pub subject: String,
    /// The tag fields of [`Self::subject`], as `(name, value)` pairs.
    pub fields: Vec<(String, String)>,
    /// Why there are no fields (a stream, an unreadable file, a file without
    /// tags). `None` with empty `fields` means "no tags".
    pub message: Option<String>,
}

/// The extended-info box's body lines.
///
/// The rule (own choice, see [`ExtendedInfo`]): the first line names how
/// many entries are selected, the second names the entry the tags belong
/// to, and the rest are the tag fields — or the message that says why there
/// are none. A multi-selection says that it shows the first entry, so the
/// summary is not read as the tags of all of them.
pub fn extended_info_lines(info: &ExtendedInfo) -> Vec<String> {
    let mut lines = Vec::new();
    if info.selected == 1 {
        lines.push("1 entry selected".to_string());
    } else {
        lines.push(format!(
            "{} entries selected (showing the first)",
            info.selected
        ));
    }
    lines.push(info.subject.clone());
    if !info.fields.is_empty() {
        lines.extend(
            info.fields
                .iter()
                .map(|(name, value)| format!("{name}: {value}")),
        );
    } else {
        lines.push(
            info.message
                .clone()
                .unwrap_or_else(|| "the file has no tags".to_string()),
        );
    }
    lines
}

/// The title of the edit-entry dialog. Own choice: no spec fixes a dialog
/// title (S04 open).
pub const EDIT_ENTRY_TITLE: &str = "Edit playlist entry";
/// Client size of the edit-entry dialog: the nine field lines, the hint line
/// and the title strip. Own choice (no spec fixes dialog geometry, S04 open).
pub const EDIT_ENTRY_CLIENT_SIZE: (u32, u32) = (320, 150);
/// The hint the edit-entry dialog shows when every field is valid. Own
/// choice.
pub const EDIT_ENTRY_HINT: &str = "Tab moves between the fields, Enter saves, Escape cancels.";
/// The longest text the edit-entry dialog keeps in a free-text field. Own
/// choice: the reference's limits are unmeasured (S04 open).
pub const MAX_EDIT_ENTRY_TEXT_LEN: usize = 256;
/// The longest genre text the edit-entry dialog keeps. Own choice.
pub const MAX_EDIT_ENTRY_GENRE_LEN: usize = 64;
/// The longest year text the edit-entry dialog keeps. Own choice: a `u32`
/// fits in ten digits, and the range check does the rest.
pub const MAX_EDIT_ENTRY_YEAR_LEN: usize = 10;
/// The longest track or disc text the edit-entry dialog keeps. Own choice:
/// two numbers and the separator between them.
pub const MAX_EDIT_ENTRY_NUMBER_LEN: usize = 21;
/// The smallest year the edit-entry dialog accepts. Own choice: the writer
/// takes every year up to [`MAX_EDIT_ENTRY_YEAR`], and the reader can produce
/// a zero year (`0000`).
pub const MIN_EDIT_ENTRY_YEAR: u32 = 0;
/// The largest year the edit-entry dialog accepts. Own choice, and the one
/// the writer enforces: `rustamp-meta` refuses a year above 9999.
pub const MAX_EDIT_ENTRY_YEAR: u32 = 9999;
/// The largest track or disc number the edit-entry dialog accepts. Own
/// choice.
pub const MAX_EDIT_ENTRY_NUMBER: u32 = 9999;
/// The key the total of a disc is carried in.
///
/// [`TrackMetadata`] has no `disc_total` field, so the total of a `1/2` disc
/// rides in the `extra` map under this key — the key `rustamp-meta`'s reader
/// parks it in. A read → edit → write cycle therefore keeps it for the tag
/// formats that have a field for it (the Vorbis family); the other formats
/// refuse it, and the write report names it.
pub const DISC_TOTAL_KEY: &str = "DISCTOTAL";

/// The editable tag fields of the edit-entry dialog, in the order the dialog
/// shows them.
///
/// The list is the tag fields [`TrackMetadata`] carries that the writer can
/// store: title, artist, album, album artist, year, genre, track, disc and
/// comment. What the contract does not carry for a tag
/// ([`TrackMetadata::duration_ms`], [`TrackMetadata::bitrate_kbps`]) and the
/// free-form `extra` map are not editable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEntryField {
    /// The track title.
    Title,
    /// The artist.
    Artist,
    /// The album.
    Album,
    /// The album artist.
    AlbumArtist,
    /// The release year.
    Year,
    /// The genre.
    Genre,
    /// The track number, with the total when the file carries one.
    Track,
    /// The disc number, with the total when the file carries one.
    Disc,
    /// The comment.
    Comment,
}

impl EditEntryField {
    /// Every field, in the order the dialog shows them.
    pub const ALL: [EditEntryField; 9] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Year,
        Self::Genre,
        Self::Track,
        Self::Disc,
        Self::Comment,
    ];

    /// The label the dialog shows in front of the field.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::AlbumArtist => "Album artist",
            Self::Year => "Year",
            Self::Genre => "Genre",
            Self::Track => "Track",
            Self::Disc => "Disc",
            Self::Comment => "Comment",
        }
    }

    /// The longest text the field keeps.
    ///
    /// Own choice (the reference's limits are unmeasured, S04 open): the
    /// free-text fields keep [`MAX_EDIT_ENTRY_TEXT_LEN`] characters, the
    /// genre [`MAX_EDIT_ENTRY_GENRE_LEN`], the year [`MAX_EDIT_ENTRY_YEAR_LEN`]
    /// (a `u32` fits in ten digits; the range check does the rest) and the
    /// number fields [`MAX_EDIT_ENTRY_NUMBER_LEN`] (two numbers and the
    /// separator).
    pub const fn max_len(self) -> usize {
        match self {
            Self::Title | Self::Artist | Self::Album | Self::AlbumArtist | Self::Comment => {
                MAX_EDIT_ENTRY_TEXT_LEN
            }
            Self::Genre => MAX_EDIT_ENTRY_GENRE_LEN,
            Self::Year => MAX_EDIT_ENTRY_YEAR_LEN,
            Self::Track | Self::Disc => MAX_EDIT_ENTRY_NUMBER_LEN,
        }
    }

    /// The field's index in [`Self::ALL`].
    const fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Artist => 1,
            Self::Album => 2,
            Self::AlbumArtist => 3,
            Self::Year => 4,
            Self::Genre => 5,
            Self::Track => 6,
            Self::Disc => 7,
            Self::Comment => 8,
        }
    }
}

/// A number with an optional total: the shape of a track (`3` or `3/12`) and
/// of a disc (`1` or `1/2`) in every tag format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditEntryNumber {
    /// The number itself.
    pub number: u32,
    /// The total, when the field carried one.
    pub total: Option<u32>,
}

/// What the edit-entry dialog does to one tag field.
///
/// The type parameter is the value the field carries: the text for the
/// free-text fields, the year for the year field and an [`EditEntryNumber`]
/// for the track and the disc field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditEntryChange<T> {
    /// The field was not touched: the file keeps the value it has.
    Unchanged,
    /// The field was edited: the file gets this value.
    Edited(T),
}

impl<T> EditEntryChange<T> {
    /// Whether the field was not touched.
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

impl<T> Default for EditEntryChange<T> {
    /// The change of an untouched field.
    fn default() -> Self {
        Self::Unchanged
    }
}

/// The changed fields of the edit-entry dialog: the save request.
///
/// Only the fields the user edited carry a value; every other field is
/// [`EditEntryChange::Unchanged`]. The app hands the request to the tag
/// writer (through [`Self::to_metadata`]), so the fields the user did not
/// touch keep the value the file has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditEntryChanges {
    /// The title.
    pub title: EditEntryChange<String>,
    /// The artist.
    pub artist: EditEntryChange<String>,
    /// The album.
    pub album: EditEntryChange<String>,
    /// The album artist.
    pub album_artist: EditEntryChange<String>,
    /// The year.
    pub year: EditEntryChange<u32>,
    /// The genre.
    pub genre: EditEntryChange<String>,
    /// The track number.
    pub track: EditEntryChange<EditEntryNumber>,
    /// The disc number.
    pub disc: EditEntryChange<EditEntryNumber>,
    /// The comment.
    pub comment: EditEntryChange<String>,
}

impl EditEntryChanges {
    /// Whether no field changed.
    pub fn is_empty(&self) -> bool {
        self.title.is_unchanged()
            && self.artist.is_unchanged()
            && self.album.is_unchanged()
            && self.album_artist.is_unchanged()
            && self.year.is_unchanged()
            && self.genre.is_unchanged()
            && self.track.is_unchanged()
            && self.disc.is_unchanged()
            && self.comment.is_unchanged()
    }

    /// The patch the app hands to the tag writer.
    ///
    /// Only the changed fields are set; the rest stay `None`, which the
    /// writer reads as "leave this tag as it is" (`write_tags` keeps every
    /// field the caller does not set).
    ///
    /// Two limits of the writer show up here, and they are the writer's, not
    /// the dialog's:
    ///
    /// * A value the writer cannot clear keeps the value the file has.
    ///   Dropping the total of a `3/12` track leaves the total in the file,
    ///   and a number the user empties keeps the number the file has (a
    ///   number has no empty value).
    /// * The disc total has no field in [`TrackMetadata`], so it rides in the
    ///   `extra` map under [`DISC_TOTAL_KEY`] — the key `rustamp-meta`'s
    ///   reader parks it in.
    pub fn to_metadata(&self) -> TrackMetadata {
        let mut metadata = TrackMetadata::default();
        if let EditEntryChange::Edited(value) = &self.title {
            metadata.title = Some(value.clone());
        }
        if let EditEntryChange::Edited(value) = &self.artist {
            metadata.artist = Some(value.clone());
        }
        if let EditEntryChange::Edited(value) = &self.album {
            metadata.album = Some(value.clone());
        }
        if let EditEntryChange::Edited(value) = &self.album_artist {
            metadata.album_artist = Some(value.clone());
        }
        if let EditEntryChange::Edited(year) = self.year {
            metadata.year = Some(year);
        }
        if let EditEntryChange::Edited(value) = &self.genre {
            metadata.genre = Some(value.clone());
        }
        if let EditEntryChange::Edited(number) = &self.track {
            metadata.track_no = Some(number.number);
            metadata.track_total = number.total;
        }
        if let EditEntryChange::Edited(number) = &self.disc {
            metadata.disc_no = Some(number.number);
            if let Some(total) = number.total {
                metadata
                    .extra
                    .insert(DISC_TOTAL_KEY.to_owned(), total.to_string());
            }
        }
        if let EditEntryChange::Edited(value) = &self.comment {
            metadata.comment = Some(value.clone());
        }
        metadata
    }
}

/// Why a field of the edit-entry dialog does not validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEntryError {
    /// The year is not a number between [`MIN_EDIT_ENTRY_YEAR`] and
    /// [`MAX_EDIT_ENTRY_YEAR`].
    InvalidYear,
    /// The track or the disc field is neither `n` nor `n/total`.
    InvalidNumber,
}

/// The field of the edit-entry dialog that does not validate, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditEntryInvalid {
    /// The field that does not validate.
    pub field: EditEntryField,
    /// Why it does not validate.
    pub error: EditEntryError,
}

impl EditEntryInvalid {
    /// The message the dialog shows for the problem.
    pub fn message(&self) -> String {
        match self.error {
            EditEntryError::InvalidYear => format!(
                "the year must be a number between {MIN_EDIT_ENTRY_YEAR} and {MAX_EDIT_ENTRY_YEAR}"
            ),
            EditEntryError::InvalidNumber => "the number must be `n` or `n/total`".to_string(),
        }
    }
}

/// What a key press did in the edit-entry dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditEntryOutcome {
    /// The key was not consumed by the dialog.
    Unhandled,
    /// The key changed the dialog: text was typed, a character was deleted or
    /// the focus moved.
    Edited,
    /// `Enter`: the dialog can be saved, and the request carries the changed
    /// fields.
    Save(EditEntryChanges),
    /// `Enter`: a field does not validate, so the dialog stays open.
    Invalid(EditEntryInvalid),
    /// `Escape`: the dialog is cancelled and nothing is written.
    Cancel,
}

/// One field of the edit-entry dialog: the text it holds now and the text it
/// was opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditEntryValue {
    current: String,
    original: String,
}

/// The edit-entry dialog (S03 §2 `40255` "Edit the playlist entry").
///
/// The dialog edits the **tags of one playlist entry**: it shows a field per
/// tag [`TrackMetadata`] carries, pre-filled with the value the file has, and
/// reports the fields the user changed. It does **not** write anything: the
/// app reads the tags of the selected entry, opens the dialog with them and
/// hands the save request to the tag writer (see
/// [`EditEntryChanges::to_metadata`]).
///
/// S03 §2 gives the command id and the chord only, so everything about the
/// dialog — its fields, its layout and its validation — is an **own choice**
/// (the reference's dialog is unmeasured, S04 open). The rules it follows:
///
/// * The focus starts on the first field. `Tab` moves it to the next field
///   and `Shift`+`Tab` to the previous one; both wrap around.
/// * The free-text fields take any text. The year takes a number between
///   [`MIN_EDIT_ENTRY_YEAR`] and [`MAX_EDIT_ENTRY_YEAR`]. The track and the
///   disc field take `n` or `n/total`.
/// * A field keeps at most [`EditEntryField::max_len`] characters.
/// * `Enter` produces the save request and `Escape` cancels. A field that
///   does not validate keeps the dialog open, so the user can fix it.
///
/// The dialog belongs to [`DialogKind::EditEntry`], the app's window
/// inventory: the app opens the window, fills the dialog with the tags it
/// read and hands the save request to the tag writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditEntryDialog {
    values: Vec<EditEntryValue>,
    focus: usize,
}

impl EditEntryDialog {
    /// Opens the dialog on `metadata`: every field starts with the value the
    /// file carries, and the focus starts on the first field.
    pub fn new(metadata: &TrackMetadata) -> Self {
        let values = EditEntryField::ALL
            .iter()
            .map(|field| {
                let text = edit_entry_initial_text(*field, metadata);
                EditEntryValue {
                    current: text.clone(),
                    original: text,
                }
            })
            .collect();
        Self { values, focus: 0 }
    }

    /// The index of the focused field.
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The focused field.
    pub fn focused_field(&self) -> EditEntryField {
        EditEntryField::ALL[self.focus]
    }

    /// Moves the focus to the next field (wrapping around).
    pub fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.values.len();
    }

    /// Moves the focus to the previous field (wrapping around).
    pub fn focus_prev(&mut self) {
        self.focus = if self.focus == 0 {
            self.values.len() - 1
        } else {
            self.focus - 1
        };
    }

    /// The text `field` holds.
    pub fn field_text(&self, field: EditEntryField) -> &str {
        &self.values[field.index()].current
    }

    /// Replaces the text of `field` (clamped to the field's cap).
    ///
    /// Typing goes through [`type_text`](Self::type_text) instead; this
    /// replaces the whole field at once (embedding and tests).
    pub fn set_field_text(&mut self, field: EditEntryField, text: &str) {
        let value = &mut self.values[field.index()];
        value.current = text.chars().take(field.max_len()).collect();
    }

    /// Types `text` into the focused field.
    ///
    /// The text is what the OS produced for the key press (layout-aware, with
    /// case and punctuation). Control characters are dropped: `Tab`, `Enter`,
    /// `Backspace` and `Escape` take the [`key`](Self::key) path. Returns
    /// whether anything was typed.
    pub fn type_text(&mut self, text: &str) -> bool {
        let mut typed = false;
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            self.push_char(ch);
            typed = true;
        }
        typed
    }

    /// Deletes the last character of the focused field. Returns whether a
    /// character was deleted.
    pub fn backspace(&mut self) -> bool {
        self.values[self.focus].current.pop().is_some()
    }

    /// Handles one key press.
    ///
    /// `Tab` moves the focus (`Shift`+`Tab` moves it back), `Backspace`
    /// deletes the last character of the focused field, the letter, digit and
    /// space keys type into it, `Enter` saves and `Escape` cancels.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> EditEntryOutcome {
        match key {
            Key::Tab => {
                if modifiers.shift() {
                    self.focus_prev();
                } else {
                    self.focus_next();
                }
                EditEntryOutcome::Edited
            }
            Key::Backspace => {
                if self.backspace() {
                    EditEntryOutcome::Edited
                } else {
                    EditEntryOutcome::Unhandled
                }
            }
            Key::Letter(letter) => {
                self.push_char(letter as char);
                EditEntryOutcome::Edited
            }
            Key::Digit(digit) => {
                self.push_char((b'0' + digit % 10) as char);
                EditEntryOutcome::Edited
            }
            Key::Numpad(digit) => {
                self.push_char((b'0' + digit % 10) as char);
                EditEntryOutcome::Edited
            }
            Key::Space => {
                self.push_char(' ');
                EditEntryOutcome::Edited
            }
            Key::Enter => match self.save_request() {
                Ok(changes) => EditEntryOutcome::Save(changes),
                Err(invalid) => EditEntryOutcome::Invalid(invalid),
            },
            Key::Escape => EditEntryOutcome::Cancel,
            _ => EditEntryOutcome::Unhandled,
        }
    }

    /// The save request: the fields the user changed.
    ///
    /// `Err` when a field does not validate: the dialog stays open so the
    /// user can fix it, and there is nothing to write yet.
    pub fn save_request(&self) -> Result<EditEntryChanges, EditEntryInvalid> {
        Ok(EditEntryChanges {
            title: self.changed_text(EditEntryField::Title),
            artist: self.changed_text(EditEntryField::Artist),
            album: self.changed_text(EditEntryField::Album),
            album_artist: self.changed_text(EditEntryField::AlbumArtist),
            year: self.changed_year()?,
            genre: self.changed_text(EditEntryField::Genre),
            track: self.changed_number(EditEntryField::Track)?,
            disc: self.changed_number(EditEntryField::Disc)?,
            comment: self.changed_text(EditEntryField::Comment),
        })
    }

    /// The first field that does not validate, in the order the dialog shows
    /// the fields.
    pub fn validation_error(&self) -> Option<EditEntryInvalid> {
        EditEntryField::ALL.iter().find_map(|field| {
            self.field_error(*field).map(|error| EditEntryInvalid {
                field: *field,
                error,
            })
        })
    }

    /// The problem with `field`, if it has one.
    fn field_error(&self, field: EditEntryField) -> Option<EditEntryError> {
        let text = self.values[field.index()].current.trim();
        if text.is_empty() {
            // An empty number field keeps the value the file has: a number
            // has no empty value.
            return None;
        }
        match field {
            EditEntryField::Year => parse_edit_year(text)
                .is_none()
                .then_some(EditEntryError::InvalidYear),
            EditEntryField::Track | EditEntryField::Disc => parse_edit_number(text)
                .is_none()
                .then_some(EditEntryError::InvalidNumber),
            _ => None,
        }
    }

    /// Appends one character to the focused field (a no-op at the cap).
    fn push_char(&mut self, ch: char) {
        let field = self.focused_field();
        let value = &mut self.values[self.focus];
        if value.current.chars().count() < field.max_len() {
            value.current.push(ch);
        }
    }

    /// The change of a free-text field.
    fn changed_text(&self, field: EditEntryField) -> EditEntryChange<String> {
        let value = &self.values[field.index()];
        if value.current == value.original {
            EditEntryChange::Unchanged
        } else {
            EditEntryChange::Edited(value.current.clone())
        }
    }

    /// The change of the year field.
    fn changed_year(&self) -> Result<EditEntryChange<u32>, EditEntryInvalid> {
        let value = &self.values[EditEntryField::Year.index()];
        if value.current == value.original {
            return Ok(EditEntryChange::Unchanged);
        }
        let text = value.current.trim();
        if text.is_empty() {
            // A year has no empty value: the field keeps the year the file
            // has.
            return Ok(EditEntryChange::Unchanged);
        }
        match parse_edit_year(text) {
            Some(year) => Ok(EditEntryChange::Edited(year)),
            None => Err(EditEntryInvalid {
                field: EditEntryField::Year,
                error: EditEntryError::InvalidYear,
            }),
        }
    }

    /// The change of the track or the disc field.
    fn changed_number(
        &self,
        field: EditEntryField,
    ) -> Result<EditEntryChange<EditEntryNumber>, EditEntryInvalid> {
        let value = &self.values[field.index()];
        if value.current == value.original {
            return Ok(EditEntryChange::Unchanged);
        }
        let text = value.current.trim();
        if text.is_empty() {
            // A number has no empty value: the field keeps the number the
            // file has.
            return Ok(EditEntryChange::Unchanged);
        }
        match parse_edit_number(text) {
            Some(number) => Ok(EditEntryChange::Edited(number)),
            None => Err(EditEntryInvalid {
                field,
                error: EditEntryError::InvalidNumber,
            }),
        }
    }
}

/// The edit-entry dialog's body lines.
///
/// One line per field, in the order the dialog shows the fields, with the
/// focused field marked (`> `) and an empty field shown as a dash. The last
/// line names the keys, or the field that does not validate.
pub fn edit_entry_lines(dialog: &EditEntryDialog) -> Vec<String> {
    let mut lines = Vec::with_capacity(EditEntryField::ALL.len() + 1);
    for field in EditEntryField::ALL {
        let marker = if field == dialog.focused_field() {
            "> "
        } else {
            "  "
        };
        let text = dialog.field_text(field);
        let shown = if text.is_empty() { "-" } else { text };
        lines.push(format!("{marker}{}: {shown}", field.label()));
    }
    match dialog.validation_error() {
        Some(invalid) => lines.push(format!("{}: {}", invalid.field.label(), invalid.message())),
        None => lines.push(EDIT_ENTRY_HINT.to_string()),
    }
    lines
}

/// The text a field starts with: the value the file carries.
fn edit_entry_initial_text(field: EditEntryField, metadata: &TrackMetadata) -> String {
    match field {
        EditEntryField::Title => metadata.title.clone().unwrap_or_default(),
        EditEntryField::Artist => metadata.artist.clone().unwrap_or_default(),
        EditEntryField::Album => metadata.album.clone().unwrap_or_default(),
        EditEntryField::AlbumArtist => metadata.album_artist.clone().unwrap_or_default(),
        EditEntryField::Year => metadata
            .year
            .map(|year| year.to_string())
            .unwrap_or_default(),
        EditEntryField::Genre => metadata.genre.clone().unwrap_or_default(),
        EditEntryField::Track => edit_entry_number_text(metadata.track_no, metadata.track_total),
        EditEntryField::Disc => {
            edit_entry_number_text(metadata.disc_no, edit_entry_disc_total(metadata))
        }
        EditEntryField::Comment => metadata.comment.clone().unwrap_or_default(),
    }
}

/// The text of a number field: `n` or `n/total`.
fn edit_entry_number_text(number: Option<u32>, total: Option<u32>) -> String {
    match (number, total) {
        (Some(number), Some(total)) => format!("{number}/{total}"),
        (Some(number), None) => number.to_string(),
        // A total without a number has no place in the field's shape.
        (None, _) => String::new(),
    }
}

/// The disc total the metadata carries.
///
/// [`TrackMetadata`] has no `disc_total` field, so the total of a `1/2` disc
/// rides in the `extra` map under [`DISC_TOTAL_KEY`] — the key
/// `rustamp-meta`'s reader parks it in.
fn edit_entry_disc_total(metadata: &TrackMetadata) -> Option<u32> {
    metadata.extra.get(DISC_TOTAL_KEY)?.parse().ok()
}

/// Parses the year field's text: a plain decimal number between
/// [`MIN_EDIT_ENTRY_YEAR`] and [`MAX_EDIT_ENTRY_YEAR`].
fn parse_edit_year(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: u32 = text.parse().ok()?;
    (MIN_EDIT_ENTRY_YEAR..=MAX_EDIT_ENTRY_YEAR)
        .contains(&year)
        .then_some(year)
}

/// Parses a track or disc field's text: `n` or `n/total`.
fn parse_edit_number(text: &str) -> Option<EditEntryNumber> {
    if text.is_empty() {
        return None;
    }
    let mut parts = text.split('/');
    let number = parse_edit_count(parts.next()?)?;
    let total = match parts.next() {
        Some(part) => Some(parse_edit_count(part)?),
        None => None,
    };
    if parts.next().is_some() {
        // A second separator is not a number pair.
        return None;
    }
    Some(EditEntryNumber { number, total })
}

/// Parses one part of a number pair: a plain decimal number up to
/// [`MAX_EDIT_ENTRY_NUMBER`].
fn parse_edit_count(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: u32 = text.parse().ok()?;
    (value <= MAX_EDIT_ENTRY_NUMBER).then_some(value)
}

/// The note lines the app surfaces after reading a playlist file.
///
/// `loaded` is the number of surviving entries, `skips` the per-entry skip
/// reasons in order (the app passes `rustamp_playlist::SkippedEntry`
/// `reason`s, which this crate must not name so it stays free of a playlist
/// dependency). The first line always names the counts and the file; one
/// `skipped: <reason>` line follows per skip. The app truncates through its
/// own note cap when the list is long.
pub fn playlist_load_summary_lines(path: &Path, loaded: usize, skips: &[String]) -> Vec<String> {
    let mut lines = vec![format!(
        "loaded {loaded} track{} ({} skipped) from {}",
        if loaded == 1 { "" } else { "s" },
        skips.len(),
        path.display()
    )];
    lines.extend(skips.iter().map(|reason| format!("skipped: {reason}")));
    lines
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
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            },
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            },
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            },
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            },
            DialogKind::SkinBrowser,
            DialogKind::Preferences(PrefsPage::VisPlugin),
            DialogKind::Preferences(PrefsPage::VisOptions),
        ] {
            let command = opening_command(dialog).expect("the dialog has a command");
            assert_eq!(dialog_for_command(&command), Some(dialog), "{command:?}");
        }
    }

    #[test]
    fn the_extended_info_box_has_no_command_and_no_entry_in_the_command_map() {
        // The playlist's `Ctrl+Alt+E` action opens it (S03 §2 `40293`):
        // reading a file's tags is not a player action, so no core command
        // exists (RFC 0005 §9) and the app opens the box itself.
        assert_eq!(opening_command(DialogKind::ExtendedInfo), None);
        assert_eq!(
            dialog_for_command(&Command::OpenFileInfo),
            Some(DialogKind::FileInfo),
            "the file-info box is a different box"
        );
    }

    #[test]
    fn the_edit_entry_box_has_no_command_either() {
        // The playlist's `Ctrl+E` action opens it (S03 §2 `40255`): editing
        // a file's tags is not a player action, so no core command exists
        // (RFC 0005 §9) and the app opens the box itself.
        assert_eq!(opening_command(DialogKind::EditEntry), None);
        assert_eq!(
            DialogKind::EditEntry.title(),
            EDIT_ENTRY_TITLE,
            "the box carries the title the app gives its window"
        );
        assert_eq!(DialogKind::EditEntry.client_size(), EDIT_ENTRY_CLIENT_SIZE);
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
            DialogKind::Jump(JumpTarget::File).client_size(),
            JUMP_FILE_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::LoadPlaylist.client_size(),
            LOAD_PLAYLIST_CLIENT_SIZE
        );
        for kind in [
            OpenMediaKind::Files,
            OpenMediaKind::Directory,
            OpenMediaKind::Location,
        ] {
            assert_eq!(
                DialogKind::OpenMedia { kind, play: true }.client_size(),
                OPEN_MEDIA_CLIENT_SIZE,
                "{kind:?}"
            );
        }
        assert_eq!(
            DialogKind::SkinBrowser.client_size(),
            SKIN_BROWSER_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).client_size(),
            PREFS_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::ExtendedInfo.client_size(),
            EXTENDED_INFO_CLIENT_SIZE
        );
        assert_eq!(DialogKind::EditEntry.client_size(), EDIT_ENTRY_CLIENT_SIZE);
        for size in [
            ABOUT_CLIENT_SIZE,
            FILE_INFO_CLIENT_SIZE,
            EXTENDED_INFO_CLIENT_SIZE,
            EDIT_ENTRY_CLIENT_SIZE,
            JUMP_CLIENT_SIZE,
            LOAD_PLAYLIST_CLIENT_SIZE,
            OPEN_MEDIA_CLIENT_SIZE,
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
    fn the_extended_info_box_names_the_selection_and_the_fields() {
        // Own choice (S03 §2 gives the id and the chord only): the count
        // first, then the entry the fields belong to, then the fields.
        let info = ExtendedInfo {
            selected: 1,
            subject: r"C:\Music\song.mp3".to_string(),
            fields: vec![
                ("Title".to_string(), "A Song".to_string()),
                ("Artist".to_string(), "An Artist".to_string()),
            ],
            message: None,
        };
        assert_eq!(
            extended_info_lines(&info),
            vec![
                "1 entry selected",
                r"C:\Music\song.mp3",
                "Title: A Song",
                "Artist: An Artist",
            ]
        );

        // A multi-selection says that it shows the first entry.
        let many = ExtendedInfo {
            selected: 3,
            ..info.clone()
        };
        let lines = extended_info_lines(&many);
        assert_eq!(lines[0], "3 entries selected (showing the first)");
        assert_eq!(lines[1], r"C:\Music\song.mp3");

        // Without fields the message says why there are none.
        let empty = ExtendedInfo {
            selected: 1,
            subject: "http://example.com/stream".to_string(),
            fields: Vec::new(),
            message: Some("a stream has no local file to read tags from".to_string()),
        };
        assert_eq!(
            extended_info_lines(&empty),
            vec![
                "1 entry selected",
                "http://example.com/stream",
                "a stream has no local file to read tags from",
            ]
        );

        // A snapshot without a message says the plain thing.
        let untagged = ExtendedInfo {
            selected: 1,
            subject: r"C:\Music\plain.mp3".to_string(),
            fields: Vec::new(),
            message: None,
        };
        assert_eq!(
            extended_info_lines(&untagged).last().map(String::as_str),
            Some("the file has no tags")
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
    fn the_jump_to_file_box_takes_any_text_and_bounds_its_selection() {
        let mut manager = DialogManager::new();
        manager.open(DialogKind::Jump(JumpTarget::File));
        assert!(manager.accepts_text());
        assert!(manager.type_text("Gamma: 3"));
        assert_eq!(manager.jump_text(), "Gamma: 3");
        assert!(manager.key(Key::Letter(b'X')));
        assert!(manager.key(Key::Space));
        assert_eq!(manager.jump_text(), "Gamma: 3X ");

        // Moves saturate at the rows the app reported; typing resets them.
        manager.set_jump_rows(3);
        for _ in 0..5 {
            assert!(manager.key(Key::Down));
        }
        assert_eq!(manager.jump_index(), 2);
        assert!(manager.key(Key::Up));
        assert_eq!(manager.jump_index(), 1);
        assert!(manager.type_text("y"));
        assert_eq!(manager.jump_index(), 0);

        // The limit is the file box's own, longer than the time box's.
        manager.set_jump_text(&"a".repeat(100));
        assert_eq!(manager.jump_text().chars().count(), MAX_JUMP_FILE_LEN);
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert!(!manager.type_text("a"));
        assert!(!manager.key(Key::Down));
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
    fn confirming_the_load_playlist_dialog_yields_no_path_without_input() {
        // RFC 0010: load playlist confirmation produces a path (not a core command).
        // Without an input or staged path, confirming yields None.
        let mut manager = DialogManager::new();
        assert_eq!(manager.confirm_load_playlist_path(), None);
        manager.open(DialogKind::LoadPlaylist);
        assert_eq!(manager.confirm_load_playlist_path(), None);
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert_eq!(manager.confirm_load_playlist_path(), None);
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

    #[test]
    fn the_open_media_titles_are_kind_specific() {
        // The files face reuses the settled ADD title (S06 §4); the other two
        // are own choices (no spec fixes them, S04 open) and stay pinned here.
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            }
            .title(),
            "Add file(s) to playlist"
        );
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            }
            .title(),
            crate::playlist::ADD_DIALOG_TITLE
        );
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            }
            .title(),
            "Open directory"
        );
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            }
            .title(),
            "Open location"
        );
    }

    #[test]
    fn staged_paths_map_to_file_locations_in_order() {
        use rustamp_core::playlist::MediaLocation;
        let paths = vec![PathBuf::from("a.mp3"), PathBuf::from("b/c.mp3")];
        assert_eq!(
            media_items_for_paths(&paths),
            vec![
                MediaLocation::File(PathBuf::from("a.mp3")),
                MediaLocation::File(PathBuf::from("b/c.mp3")),
            ]
        );
        assert!(media_items_for_paths(&[]).is_empty());
    }

    #[test]
    fn a_location_entry_is_a_url_or_nothing() {
        use rustamp_core::playlist::MediaLocation;
        assert_eq!(
            media_item_for_url("http://example.com/stream.mp3"),
            Some(MediaLocation::Url(
                "http://example.com/stream.mp3".to_string()
            ))
        );
        // Surrounding whitespace is ignored, but the text is passed through
        // otherwise (URL syntax is unmeasured).
        assert_eq!(
            media_item_for_url("  http://example.com/x  "),
            Some(MediaLocation::Url("http://example.com/x".to_string()))
        );
        assert_eq!(media_item_for_url(""), None);
        assert_eq!(media_item_for_url("   "), None);
    }

    #[test]
    fn confirming_files_adds_the_staged_picks_with_the_play_flag() {
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        assert_eq!(manager.confirm_open_media(), None);
        // The main-window path plays; the playlist ADD path does not (RFC
        // 0005 O5: the `play: false` value is unmeasured and rides through
        // untouched — the dialog never decides it).
        for play in [true, false] {
            manager.open(DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play,
            });
            assert_eq!(manager.confirm_open_media(), None);
            manager.stage_media_paths(vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")]);
            assert_eq!(
                manager.confirm_open_media(),
                Some(Command::AddItems {
                    items: vec![
                        MediaLocation::File(PathBuf::from("a.mp3")),
                        MediaLocation::File(PathBuf::from("b.mp3")),
                    ],
                    at: None,
                    play,
                }),
                "play = {play}"
            );
        }
    }

    #[test]
    fn confirming_files_without_picks_falls_back_to_the_typed_path() {
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        assert_eq!(manager.confirm_open_media(), None);
        manager.set_media_text("typed.mp3");
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![MediaLocation::File(PathBuf::from("typed.mp3"))],
                at: None,
                play: false,
            })
        );
    }

    #[test]
    fn confirming_a_directory_adds_one_path_without_expansion() {
        // No directory expansion exists yet: the pick is added as-is (open
        // with the reference's directory semantics).
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: true,
        });
        assert_eq!(manager.confirm_open_media(), None);
        manager.stage_media_paths(vec![PathBuf::from("music"), PathBuf::from("other")]);
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![MediaLocation::File(PathBuf::from("music"))],
                at: None,
                play: true,
            })
        );
        // Without a staged pick the typed field is the directory.
        manager.stage_media_paths(Vec::new());
        manager.set_media_text("typed-dir");
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![MediaLocation::File(PathBuf::from("typed-dir"))],
                at: None,
                play: true,
            })
        );
    }

    #[test]
    fn confirming_a_location_adds_the_typed_url() {
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        });
        assert_eq!(manager.confirm_open_media(), None);
        manager.set_media_text("http://example.com/stream.mp3");
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![MediaLocation::Url(
                    "http://example.com/stream.mp3".to_string()
                )],
                at: None,
                play: true,
            })
        );
        // Blank input confirms nothing; the dialog stays open for a retry.
        manager.set_media_text("   ");
        assert_eq!(manager.confirm_open_media(), None);
    }

    #[test]
    fn confirming_an_open_media_dialog_needs_that_dialog() {
        // Another dialog open (or none) confirms nothing.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::About);
        manager.set_media_text("x.mp3");
        assert_eq!(manager.confirm_open_media(), None);
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert_eq!(manager.confirm_open_media(), None);
    }

    #[test]
    fn cancelling_an_open_media_dialog_drops_its_field_and_picks() {
        // `Escape` (or closing) clears the entry field and the staged picks
        // with the dialog, so a reopened dialog starts empty.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        manager.set_media_text("x.mp3");
        manager.stage_media_paths(vec![PathBuf::from("y.mp3")]);
        manager.close_any();
        assert_eq!(manager.media_text(), "");
        assert!(manager.staged_media_paths().is_empty());
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        assert_eq!(manager.confirm_open_media(), None);
    }

    #[test]
    fn open_media_keys_type_into_the_field_and_nothing_else() {
        // Letters, digits and spaces append to the entry field (digits still
        // type into the jump box too); arrows go nowhere on these dialogs.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        });
        assert!(manager.key(Key::Digit(8)));
        assert!(manager.key(Key::Numpad(0)));
        assert!(manager.key(Key::Space));
        assert!(!manager.key(Key::Up));
        assert!(!manager.key(Key::Down));
        assert_eq!(manager.media_text(), "80 ");
        assert!(manager.key(Key::Enter));
        assert!(manager.key(Key::Escape));
        // The same keys type nothing on the neighbouring dialogs.
        manager.open(DialogKind::About);
        assert!(!manager.key(Key::Digit(1)));
        assert!(!manager.key(Key::Space));
        manager.open(DialogKind::LoadPlaylist);
        assert!(!manager.key(Key::Digit(1)));
        assert!(!manager.key(Key::Space));
        // Letters keep the key model's upper case; exact text arrives through
        // the setter.
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        assert!(manager.key(Key::Letter(b'H')));
        assert_eq!(manager.media_text(), "H");
    }

    #[test]
    fn a_full_windows_path_can_be_typed_and_corrected() {
        // BETA-03: the OS text of each key press reaches the field with its
        // case and punctuation, so an ordinary path is typeable; Backspace
        // fixes a typo and control characters never land in the field.
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: true,
        });
        assert!(manager.accepts_text());
        for ch in r"C:\Music\song.mp33".chars() {
            assert!(manager.type_text(&ch.to_string()), "{ch:?}");
        }
        assert!(manager.key(Key::Backspace));
        // Enter's and Ctrl+V's control characters are not text.
        assert!(!manager.type_text("\r"));
        assert!(!manager.type_text("\u{16}"));
        assert_eq!(manager.media_text(), r"C:\Music\song.mp3");
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![MediaLocation::File(PathBuf::from(r"C:\Music\song.mp3"))],
                at: None,
                play: true,
            })
        );
    }

    #[test]
    fn the_jump_box_takes_typed_digits_and_colons_only() {
        let mut manager = DialogManager::new();
        manager.open(DialogKind::Jump(JumpTarget::Time));
        assert!(manager.type_text("1:3"));
        assert!(!manager.type_text("x"));
        assert!(manager.type_text("0"));
        assert_eq!(manager.jump_text(), "1:30");
        assert!(manager.key(Key::Backspace));
        assert_eq!(manager.jump_text(), "1:3");
        // Dialogs without a field take no text and no Backspace. The
        // load-playlist dialog is not one of them: it takes a typed path
        // through `type_text` (see the playlist-path tests below).
        for dialog in [DialogKind::About, DialogKind::SkinBrowser] {
            manager.open(dialog);
            assert!(!manager.accepts_text(), "{dialog:?}");
            assert!(!manager.type_text("a"), "{dialog:?}");
            assert!(!manager.key(Key::Backspace), "{dialog:?}");
        }
    }

    #[test]
    fn dropped_files_become_picks_on_the_file_faces_only() {
        use rustamp_core::playlist::MediaLocation;
        let mut manager = DialogManager::new();
        assert!(!manager.drop_media_path(PathBuf::from("a.mp3")));
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        assert!(manager.drop_media_path(PathBuf::from("a.mp3")));
        assert!(manager.drop_media_path(PathBuf::from("b.mp3")));
        assert_eq!(
            manager.confirm_open_media(),
            Some(Command::AddItems {
                items: vec![
                    MediaLocation::File(PathBuf::from("a.mp3")),
                    MediaLocation::File(PathBuf::from("b.mp3")),
                ],
                at: None,
                play: false,
            })
        );
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: true,
        });
        assert!(manager.drop_media_path(PathBuf::from("one")));
        assert!(manager.drop_media_path(PathBuf::from("two")));
        assert_eq!(manager.staged_media_paths(), [PathBuf::from("two")]);
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        });
        assert!(!manager.drop_media_path(PathBuf::from("a.mp3")));
    }

    #[test]
    fn a_playlist_path_can_be_typed_staged_or_dropped() {
        // BETA-09 (UI half): the load-playlist dialog carries a path the app
        // half turns into entries plus a skip report. Typing, staging and
        // dropping all feed the same confirm; the staged pick wins.
        let mut manager = DialogManager::new();
        manager.open(DialogKind::LoadPlaylist);
        assert!(manager.accepts_text());
        for ch in r"C:\Music\mix.m3u8".chars() {
            assert!(manager.type_text(&ch.to_string()), "{ch:?}");
        }
        assert!(manager.key(Key::Backspace));
        assert_eq!(manager.playlist_text(), r"C:\Music\mix.m3u");
        assert_eq!(
            manager.confirm_load_playlist_path(),
            Some(PathBuf::from(r"C:\Music\mix.m3u"))
        );
        // A staged pick wins over the typed field.
        manager.stage_playlist_path(PathBuf::from("staged.xspf"));
        assert_eq!(
            manager.confirm_load_playlist_path(),
            Some(PathBuf::from("staged.xspf"))
        );
        // A drop replaces the staged pick (last drop wins).
        assert!(manager.drop_playlist_path(PathBuf::from("dropped.pls")));
        assert_eq!(
            manager.staged_playlist_path(),
            Some(&PathBuf::from("dropped.pls"))
        );
        assert_eq!(
            manager.confirm_load_playlist_path(),
            Some(PathBuf::from("dropped.pls"))
        );
        // Control characters never land in the field.
        assert!(!manager.type_text("\r"));
        // Opening another dialog clears the field and the pick.
        manager.open(DialogKind::About);
        manager.open(DialogKind::LoadPlaylist);
        assert_eq!(manager.confirm_load_playlist_path(), None);
        assert!(manager.playlist_text().is_empty());
    }

    #[test]
    fn a_playlist_path_needs_its_dialog_and_a_path() {
        let mut manager = DialogManager::new();
        // No dialog open: nothing to confirm, no drop taken.
        assert_eq!(manager.confirm_load_playlist_path(), None);
        assert!(!manager.drop_playlist_path(PathBuf::from("a.m3u")));
        // The open-media dialog is a different face.
        manager.open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        });
        assert_eq!(manager.confirm_load_playlist_path(), None);
        assert!(!manager.drop_playlist_path(PathBuf::from("a.m3u")));
        // Blank text confirms nothing; surrounding whitespace is trimmed.
        manager.open(DialogKind::LoadPlaylist);
        manager.set_playlist_text("   ");
        assert_eq!(manager.confirm_load_playlist_path(), None);
        manager.set_playlist_text("  mix.m3u  ");
        assert_eq!(
            manager.confirm_load_playlist_path(),
            Some(PathBuf::from("mix.m3u"))
        );
        // The key model has no punctuation, so full paths arrive through
        // `type_text`, not `key` — but Backspace still corrects the field.
        assert!(!manager.key(Key::Digit(1)));
        assert!(!manager.key(Key::Space));
        assert!(manager.key(Key::Backspace));
        // Backspace removes exactly one character — here one of the two
        // trailing spaces the field was set with (they pin the trim on
        // confirm above), so the other trailing space remains.
        assert_eq!(manager.playlist_text(), "  mix.m3u ");
    }

    #[test]
    fn the_load_summary_names_counts_and_every_skip() {
        // BETA-09 (UI half): the lines the app surfaces for an E3 report —
        // one count line plus one line per skip reason, in order.
        let lines = playlist_load_summary_lines(
            Path::new("mix.m3u"),
            3,
            &["dangling #EXTINF".to_string(), "empty File2".to_string()],
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "loaded 3 tracks (2 skipped) from mix.m3u");
        assert_eq!(lines[1], "skipped: dangling #EXTINF");
        assert_eq!(lines[2], "skipped: empty File2");
        // A clean file still names its counts.
        let lines = playlist_load_summary_lines(Path::new("clean.pls"), 1, &[]);
        assert_eq!(lines, vec!["loaded 1 track (0 skipped) from clean.pls"]);
    }

    /// The tags of a file, as the edit-entry dialog opens on them.
    fn tagged_metadata() -> TrackMetadata {
        TrackMetadata {
            title: Some("A Song".to_string()),
            artist: Some("An Artist".to_string()),
            album: Some("The Album".to_string()),
            album_artist: Some("The Album Artist".to_string()),
            year: Some(1999),
            genre: Some("Rock".to_string()),
            comment: Some("A comment".to_string()),
            track_no: Some(3),
            track_total: Some(12),
            disc_no: Some(1),
            ..TrackMetadata::default()
        }
    }

    #[test]
    fn the_edit_entry_dialog_starts_with_the_tags_of_the_file() {
        let mut metadata = tagged_metadata();
        metadata
            .extra
            .insert(DISC_TOTAL_KEY.to_string(), "2".to_string());
        let dialog = EditEntryDialog::new(&metadata);
        // The focus starts on the first field.
        assert_eq!(dialog.focus(), 0);
        assert_eq!(dialog.focused_field(), EditEntryField::Title);
        // Every field starts with the value the file carries.
        assert_eq!(dialog.field_text(EditEntryField::Title), "A Song");
        assert_eq!(dialog.field_text(EditEntryField::Artist), "An Artist");
        assert_eq!(dialog.field_text(EditEntryField::Album), "The Album");
        assert_eq!(
            dialog.field_text(EditEntryField::AlbumArtist),
            "The Album Artist"
        );
        assert_eq!(dialog.field_text(EditEntryField::Year), "1999");
        assert_eq!(dialog.field_text(EditEntryField::Genre), "Rock");
        assert_eq!(dialog.field_text(EditEntryField::Track), "3/12");
        assert_eq!(dialog.field_text(EditEntryField::Disc), "1/2");
        assert_eq!(dialog.field_text(EditEntryField::Comment), "A comment");
        // Nothing changed yet: the request is empty.
        let changes = dialog.save_request().expect("the fields validate");
        assert!(changes.is_empty());
    }

    #[test]
    fn typing_and_backspace_edit_the_focused_field() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        assert!(dialog.type_text("A Song"));
        assert_eq!(dialog.field_text(EditEntryField::Title), "A Song");
        assert!(dialog.backspace());
        assert_eq!(dialog.field_text(EditEntryField::Title), "A Son");
        // The other fields are untouched.
        assert_eq!(dialog.field_text(EditEntryField::Artist), "");
        // The key path types what the key model stores (upper-case letters).
        assert_eq!(
            dialog.key(Key::Letter(b'B'), Modifiers::NONE),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.field_text(EditEntryField::Title), "A SonB");
        assert_eq!(
            dialog.key(Key::Digit(4), Modifiers::NONE),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.field_text(EditEntryField::Title), "A SonB4");
        // Backspace on an empty field changes nothing.
        dialog.set_field_text(EditEntryField::Title, "");
        assert!(!dialog.backspace());
        assert_eq!(
            dialog.key(Key::Backspace, Modifiers::NONE),
            EditEntryOutcome::Unhandled
        );
        // Control characters are not text.
        assert!(!dialog.type_text("\r"));
    }

    #[test]
    fn tab_and_shift_tab_move_the_focus() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        assert_eq!(dialog.focused_field(), EditEntryField::Title);
        assert_eq!(
            dialog.key(Key::Tab, Modifiers::NONE),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.focused_field(), EditEntryField::Artist);
        assert_eq!(
            dialog.key(Key::Tab, Modifiers::SHIFT),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.focused_field(), EditEntryField::Title);
        // The focus wraps around at both ends.
        assert_eq!(
            dialog.key(Key::Tab, Modifiers::SHIFT),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.focused_field(), EditEntryField::Comment);
        assert_eq!(
            dialog.key(Key::Tab, Modifiers::NONE),
            EditEntryOutcome::Edited
        );
        assert_eq!(dialog.focused_field(), EditEntryField::Title);
    }

    #[test]
    fn the_year_field_takes_a_number_in_the_sane_range() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        dialog.set_field_text(EditEntryField::Year, "1999");
        assert!(dialog.validation_error().is_none());
        assert!(dialog.save_request().is_ok());
        for bad in ["abcd", "19x9", "10000", "-1", "1.5", "99999999999"] {
            dialog.set_field_text(EditEntryField::Year, bad);
            let invalid = dialog
                .validation_error()
                .expect("the year does not validate");
            assert_eq!(invalid.field, EditEntryField::Year, "{bad}");
            assert_eq!(invalid.error, EditEntryError::InvalidYear, "{bad}");
            assert!(dialog.save_request().is_err(), "{bad}");
        }
    }

    #[test]
    fn the_number_fields_take_a_number_or_a_number_pair() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        for good in ["3", "3/12", "0"] {
            dialog.set_field_text(EditEntryField::Track, good);
            assert!(dialog.validation_error().is_none(), "{good}");
            assert!(dialog.save_request().is_ok(), "{good}");
        }
        for bad in ["x", "3/", "/12", "3/12/4", "3.5", "3/99999"] {
            dialog.set_field_text(EditEntryField::Track, bad);
            let invalid = dialog
                .validation_error()
                .expect("the track does not validate");
            assert_eq!(invalid.field, EditEntryField::Track, "{bad}");
            assert_eq!(invalid.error, EditEntryError::InvalidNumber, "{bad}");
        }
    }

    #[test]
    fn the_save_request_carries_only_the_changed_fields() {
        let mut dialog = EditEntryDialog::new(&tagged_metadata());
        dialog.set_field_text(EditEntryField::Title, "Another Song");
        dialog.set_field_text(EditEntryField::Track, "4/13");
        let changes = dialog.save_request().expect("the fields validate");
        assert_eq!(
            changes.title,
            EditEntryChange::Edited("Another Song".to_string())
        );
        assert_eq!(
            changes.track,
            EditEntryChange::Edited(EditEntryNumber {
                number: 4,
                total: Some(13),
            })
        );
        // Everything else is untouched.
        assert!(changes.artist.is_unchanged());
        assert!(changes.album.is_unchanged());
        assert!(changes.album_artist.is_unchanged());
        assert!(changes.year.is_unchanged());
        assert!(changes.genre.is_unchanged());
        assert!(changes.disc.is_unchanged());
        assert!(changes.comment.is_unchanged());
        assert!(!changes.is_empty());

        // The patch the writer gets names the changed fields only: every
        // other field stays unset, which the writer reads as "leave it as it
        // is".
        let patch = changes.to_metadata();
        assert_eq!(patch.title.as_deref(), Some("Another Song"));
        assert_eq!(patch.track_no, Some(4));
        assert_eq!(patch.track_total, Some(13));
        assert_eq!(patch.artist, None);
        assert_eq!(patch.album, None);
        assert_eq!(patch.year, None);
        assert_eq!(patch.disc_no, None);
        assert!(patch.extra.is_empty());
    }

    #[test]
    fn enter_saves_the_changed_fields_and_escape_cancels() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        assert_eq!(
            dialog.key(Key::Escape, Modifiers::NONE),
            EditEntryOutcome::Cancel
        );
        dialog.set_field_text(EditEntryField::Title, "A Song");
        match dialog.key(Key::Enter, Modifiers::NONE) {
            EditEntryOutcome::Save(changes) => {
                assert_eq!(changes.title, EditEntryChange::Edited("A Song".to_string()));
                assert!(changes.artist.is_unchanged());
            }
            other => panic!("Enter saves the dialog: {other:?}"),
        }
        // A field that does not validate keeps the dialog open.
        dialog.set_field_text(EditEntryField::Year, "abcd");
        match dialog.key(Key::Enter, Modifiers::NONE) {
            EditEntryOutcome::Invalid(invalid) => {
                assert_eq!(invalid.field, EditEntryField::Year);
                assert_eq!(invalid.error, EditEntryError::InvalidYear);
            }
            other => panic!("Enter names the field that does not validate: {other:?}"),
        }
    }

    #[test]
    fn the_dialog_lines_show_every_field_and_the_focused_one() {
        let dialog = EditEntryDialog::new(&tagged_metadata());
        let lines = edit_entry_lines(&dialog);
        assert_eq!(lines.len(), EditEntryField::ALL.len() + 1);
        assert_eq!(lines[0], "> Title: A Song");
        assert_eq!(lines[1], "  Artist: An Artist");
        assert_eq!(lines[4], "  Year: 1999");
        assert_eq!(lines[6], "  Track: 3/12");
        assert_eq!(lines.last().map(String::as_str), Some(EDIT_ENTRY_HINT));
        // An empty field shows a dash.
        let empty = EditEntryDialog::new(&TrackMetadata::default());
        let lines = edit_entry_lines(&empty);
        assert_eq!(lines[1], "  Artist: -");
        // A field that does not validate replaces the hint with the problem.
        let mut broken = EditEntryDialog::new(&tagged_metadata());
        broken.set_field_text(EditEntryField::Year, "abcd");
        let lines = edit_entry_lines(&broken);
        assert_eq!(
            lines.last().map(String::as_str),
            Some("Year: the year must be a number between 0 and 9999")
        );
    }

    #[test]
    fn a_field_keeps_at_most_its_cap() {
        let mut dialog = EditEntryDialog::new(&TrackMetadata::default());
        dialog.type_text(&"a".repeat(MAX_EDIT_ENTRY_TEXT_LEN + 50));
        assert_eq!(
            dialog.field_text(EditEntryField::Title).chars().count(),
            MAX_EDIT_ENTRY_TEXT_LEN
        );
        // The genre has its own, shorter cap.
        for _ in 0..5 {
            dialog.focus_next();
        }
        assert_eq!(dialog.focused_field(), EditEntryField::Genre);
        dialog.type_text(&"g".repeat(MAX_EDIT_ENTRY_GENRE_LEN + 10));
        assert_eq!(
            dialog.field_text(EditEntryField::Genre).chars().count(),
            MAX_EDIT_ENTRY_GENRE_LEN
        );
        // The setter clamps as well.
        dialog.set_field_text(
            EditEntryField::Title,
            &"b".repeat(MAX_EDIT_ENTRY_TEXT_LEN + 10),
        );
        assert_eq!(
            dialog.field_text(EditEntryField::Title).chars().count(),
            MAX_EDIT_ENTRY_TEXT_LEN
        );
    }

    #[test]
    fn an_emptied_field_tells_the_writer_what_it_can_do() {
        let mut dialog = EditEntryDialog::new(&tagged_metadata());
        // A text field the user empties is written as an empty value.
        dialog.set_field_text(EditEntryField::Comment, "");
        let changes = dialog.save_request().expect("the fields validate");
        assert_eq!(changes.comment, EditEntryChange::Edited(String::new()));
        assert_eq!(changes.to_metadata().comment.as_deref(), Some(""));
        // A number has no empty value: the field keeps the value the file
        // has.
        dialog.set_field_text(EditEntryField::Year, "");
        let changes = dialog.save_request().expect("the fields validate");
        assert!(changes.year.is_unchanged());
        assert_eq!(changes.to_metadata().year, None);
    }

    #[test]
    fn the_dialog_is_big_enough_for_its_title_and_its_lines() {
        let dialog = EditEntryDialog::new(&tagged_metadata());
        let lines = edit_entry_lines(&dialog);
        let (width, height) = EDIT_ENTRY_CLIENT_SIZE;
        // The title strip holds the title.
        let title_width = EDIT_ENTRY_TITLE.chars().count() as u32 * webamp::TEXT_CELL.0;
        assert!(title_width <= width, "{title_width} > {width}");
        // The body holds every line.
        let body = TITLE_STRIP_HEIGHT + TEXT_PAD as u32 + lines.len() as u32 * TEXT_LINE_PX as u32;
        assert!(body <= height, "{body} > {height}");
    }

    #[test]
    fn the_disc_total_rides_in_the_extra_map() {
        // `TrackMetadata` has no `disc_total` field: the total of a `1/2`
        // disc rides in the `extra` map under `DISC_TOTAL_KEY`.
        let mut metadata = TrackMetadata {
            disc_no: Some(1),
            ..TrackMetadata::default()
        };
        metadata
            .extra
            .insert(DISC_TOTAL_KEY.to_string(), "2".to_string());
        let mut dialog = EditEntryDialog::new(&metadata);
        assert_eq!(dialog.field_text(EditEntryField::Disc), "1/2");
        // Changing the disc keeps the total in the patch.
        dialog.set_field_text(EditEntryField::Disc, "2/3");
        let changes = dialog.save_request().expect("the fields validate");
        let patch = changes.to_metadata();
        assert_eq!(patch.disc_no, Some(2));
        assert_eq!(
            patch.extra.get(DISC_TOTAL_KEY).map(String::as_str),
            Some("3")
        );
    }
}
