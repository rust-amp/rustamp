//! Headless playlist-window behaviour (spec **S06**).
//!
//! [`PlaylistWindow`] is the playlist editor's (`Winamp PE`, 275 x 116) state
//! machine. Like [`crate::behaviour::MainWindow`] it is deliberately
//! **headless**: no `winit`, no `softbuffer`, no clock and no window. It owns
//! the list view (which entries exist, which slice is visible, which entries
//! are selected) and translates the S06-settled gestures into core
//! [`Command`]s. It never opens a dialog itself; dialog *requests* are
//! returned as [`PlaylistDialog`] values (titles only) so the app layer can
//! open the OS window.
//!
//! Every rule cites its S06 section. Pixel counts below are the whole-window
//! differing-pixel counts from the sandbox runs, quoted so a reviewer can
//! match the rule to the evidence; they are not recomputed here.
//!
//! Dragging is an own choice: S06 §8 Q6 explicitly leaves reorder versus
//! selection-block semantics unresolved. A selected row drags the selection in
//! playlist order; an unselected row selects itself. Four pixels of motion
//! activate a drag. Row halves choose before/after, blank list space chooses
//! after the visible rows. Escape, leaving the list, or an order change cancels.
//! Playback and selection stay attached to entry ids; no auto-scroll is implied.

use std::collections::BTreeSet;

use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
use rustamp_core::playlist::EntryId;

use crate::shortcuts::{Key, Modifiers, ShortcutAction};

/// How many list rows are visible at once (S06 §3).
///
/// The 1501/1501 px arrow symmetry is settled; the "four rows" count itself is
/// a **visual reading** (OCR of the uncertain skin font), so the constant is
/// marked accordingly and the page step below is defined in terms of it.
pub const VISIBLE_ROWS: usize = 4;

/// One scrollbar-arrow step moves the view by one page (S06 §3, settled as a
/// pixel symmetry: up-arrow 1501 px, down-arrow 1501 px back, same bounds).
pub const PAGE_STEP_ROWS: usize = VISIBLE_ROWS;

/// The playlist editor's client size (S06 scope; S01 §1, verified).
pub const PLAYLIST_WINDOW_SIZE: (u32, u32) = (275, 116);

/// The bottom-bar buttons live at window y 92-109 (S01 §4.1, verified: the
/// sprites come from `PLEDIT.BMP` y = 86 and are drawn at window y = 92-109).
pub const BUTTON_BAND_TOP: i32 = 92;
/// Bottom-exclusive end of the bottom-bar button band (S01 §4.1).
pub const BUTTON_BAND_BOTTOM: i32 = 110;

/// The list click that selects the first visible row (S06 §2, settled: a
/// single posted click at (60,28) changes 2918 px, bbox (12,23)-(254,93)).
pub const LIST_SELECT_POINT: (i32, i32) = (60, 28);

// Own-choice drag slop in playlist client pixels (S06 §8 Q6 is open).
const DRAG_THRESHOLD: u32 = 4;

#[derive(Debug, Clone)]
struct PlaylistDrag {
    press: (i32, i32),
    ids: Vec<EntryId>,
    insertion: Option<usize>,
}

/// The five bottom-bar buttons (S06 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaylistButton {
    /// ADD: opens the add-files dialog (S06 §4).
    Add,
    /// REM: removes the selected entry (S06 §5).
    Rem,
    /// SEL: selects all visible rows (S06 §2).
    Sel,
    /// MISC: opens the `#32768` popup (S06 §4).
    Misc,
    /// LIST OPTS: opens the load-playlist dialog (S06 §4).
    ListOpts,
}

impl PlaylistButton {
    /// Every bottom-bar button, left to right.
    pub const ALL: &'static [PlaylistButton] = &[
        PlaylistButton::Add,
        PlaylistButton::Rem,
        PlaylistButton::Sel,
        PlaylistButton::Misc,
        PlaylistButton::ListOpts,
    ];
}

/// The window-relative x range of `button` at bottom-bar height, as a
/// half-open `start..end`.
///
/// Provenance (S01 §4.1/§4.2, S06 §4): the ADD (14-35), MISC (102-121) and
/// LIST OPTS (232-251) ranges are the settled posted positives at y = 92; REM
/// acts at 46 and at 47-55 (left edge settled, right edge open above 55) and
/// SEL acts at 74 only (75-83 are settled misses at y = 92). The drawn
/// buttons sit at x ≈ 11/43/72/101/228 (S01 §4.1, ±3 px ambiguous), so every
/// right edge here is **provisional**: the ranges below stop at the last
/// settled acting x and must not be read as the button's full extent.
pub const fn button_hit_range(button: PlaylistButton) -> (i32, i32) {
    match button {
        PlaylistButton::Add => (14, 36),
        PlaylistButton::Rem => (46, 56),
        PlaylistButton::Sel => (74, 75),
        PlaylistButton::Misc => (102, 122),
        PlaylistButton::ListOpts => (232, 252),
    }
}

/// Which bottom-bar button (if any) a posted click at window-relative
/// `(x, y)` hits (S01 §4.1/§4.2; ranges from [`button_hit_range`]).
///
/// Returns `None` outside the button band or on a settled miss (for example
/// the 75-83 SEL gap at y = 92, where the `WinampVis` child covers x 52-123).
pub fn hit_test_button(x: i32, y: i32) -> Option<PlaylistButton> {
    if !(BUTTON_BAND_TOP..BUTTON_BAND_BOTTOM).contains(&y) {
        return None;
    }
    PlaylistButton::ALL.iter().copied().find(|button| {
        let (start, end) = button_hit_range(*button);
        x >= start && x < end
    })
}

/// Scroll direction for [`PlaylistWindow::scroll_page`] (S06 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    /// Towards the top of the list (the up arrow at (263,81), S06 §3).
    Up,
    /// Towards the bottom of the list (the down arrow at (263,87), S06 §3).
    Down,
}

/// A playlist-editor key action (S03 §2, playlist editor table).
///
/// Implements the provisional move/scroll rows, the two provisional jump rows,
/// the menu-confirmed Enter row, the provisional `Alt+Up`/`Alt+Down` block moves,
/// the menu-confirmed `Delete` and `Ctrl+Delete` (Crop) rows, and the
/// menu-confirmed `Alt+Delete` (remove dead files), `Ctrl+Shift+Delete` /
/// `Ctrl+N` (clear the playlist) rows, and the menu-confirmed `Ctrl+Shift`
/// reorder rows (the three sorts and randomize).
///
/// The rows S03 §2 shares with the main window (S03 §1) and the global table
/// (S03 §4) — the transport keys, the window keys and the global rows — are
/// [`PlaylistKeyAction::Shared`], so the playlist editor and the main window
/// cannot drift apart on a row they share. The rows S03 §2 marks
/// **provisional** (their effect was inferred, never measured) say so in their
/// doc comment and are implemented as own choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaylistKeyAction {
    /// Up: move the selection up (`40289`, provisional).
    MoveSelectionUp,
    /// Down: move the selection down (`40290`, provisional).
    MoveSelectionDown,
    /// Home: jump to the first entry (`40264`, provisional).
    JumpToFirst,
    /// End: jump to the last entry (`40265`, provisional).
    JumpToLast,
    /// PageUp: scroll up one page (`40256`, provisional).
    PageUp,
    /// PageDown: scroll down one page (`40257`, provisional).
    PageDown,
    /// Enter: play the selected item(s) (`40184`, menu-confirmed).
    PlaySelected,
    /// Delete: remove the selected item(s) (`1034`, menu-confirmed, S06 §5).
    Delete,
    /// Ctrl+Delete: crop to the selected item(s) (`1035`, menu-confirmed).
    Crop,
    /// Alt+Delete: remove all dead files (`40222`, menu-confirmed, RFC 0018).
    RemoveDead,
    /// Ctrl+Shift+Delete or Ctrl+N: clear the playlist (`40214`,
    /// menu-confirmed; S03 §2 binds both chords to the one id).
    ClearPlaylist,
    /// Alt+Up: move the selected block up (`40215`, provisional).
    MoveBlockUp,
    /// Alt+Down: move the selected block down (`40216`, provisional).
    MoveBlockDown,
    /// Ctrl+Shift+1: sort the list by title (`40209`, menu-confirmed).
    SortByTitle,
    /// Ctrl+Shift+2: sort the list by file name (`40210`, menu-confirmed).
    SortByFileName,
    /// Ctrl+Shift+3: sort the list by path and file name (`40211`,
    /// menu-confirmed).
    SortByPathAndFileName,
    /// Ctrl+Shift+R: randomize the list (`40212`, menu-confirmed).
    Randomize,
    /// Ctrl+A: select all (`40205`, menu-confirmed).
    SelectAll,
    /// Ctrl+I: invert the selection (`40171`, menu-confirmed).
    InvertSelection,
    /// Ctrl+S: save the playlist (`40204`, menu-confirmed).
    SavePlaylist,
    /// L, Ctrl+Insert or Num0: add file(s) (`1032`, menu-confirmed).
    AddFiles,
    /// Ctrl+L, Shift+Insert or Ctrl+Num0: add a location (`1039`,
    /// menu-confirmed).
    AddLocation,
    /// Shift+L or Insert: add a directory (`1036`, menu-confirmed).
    AddDirectory,
    /// Ctrl+O: open a playlist (`40202`, menu-confirmed).
    OpenPlaylist,
    /// Ctrl+R: reverse the list (`40213`, menu-confirmed).
    Reverse,
    /// Alt+3: file info (`40208`, menu-confirmed).
    FileInfo,
    /// Alt+I: bookmark the selected item(s) (`40319`, menu-confirmed).
    BookmarkSelection,
    /// Ctrl+F4: close the playlist editor (`40224`, provisional).
    CloseEditor,
    /// Ctrl+W: toggle the editor's windowshade (`40064`).
    ///
    /// S03 §2 binds the global "toggle windowshade" id, but the editor's
    /// shade is its own (S01 §1: a shaded editor is one row tall, and the
    /// editor's title bar carries its own shade button), so the row toggles
    /// the editor and not the main window.
    ToggleEditorShade,
    /// A row whose action is one of the shared S03 actions (S03 §1/§4).
    ///
    /// The shared actions carry their own command id, so a shared row of the
    /// playlist table and the same row of the main table cannot disagree.
    Shared(ShortcutAction),
}

impl PlaylistKeyAction {
    /// The public `WM_COMMAND` id S03 §2 records for the action.
    ///
    /// The ids are interoperability facts: they are what a plugin or an
    /// external tool sends, so they belong to the spec and not to this
    /// implementation.
    pub const fn command_id(self) -> u32 {
        match self {
            Self::Shared(action) => action.command_id(),
            Self::MoveSelectionUp => 40289,
            Self::MoveSelectionDown => 40290,
            Self::JumpToFirst => 40264,
            Self::JumpToLast => 40265,
            Self::PageUp => 40256,
            Self::PageDown => 40257,
            Self::PlaySelected => 40184,
            Self::Delete => 1034,
            Self::Crop => 1035,
            Self::RemoveDead => 40222,
            Self::ClearPlaylist => 40214,
            Self::MoveBlockUp => 40215,
            Self::MoveBlockDown => 40216,
            Self::SortByTitle => 40209,
            Self::SortByFileName => 40210,
            Self::SortByPathAndFileName => 40211,
            Self::Randomize => 40212,
            Self::SelectAll => 40205,
            Self::InvertSelection => 40171,
            Self::SavePlaylist => 40204,
            Self::AddFiles => 1032,
            Self::AddLocation => 1039,
            Self::AddDirectory => 1036,
            Self::OpenPlaylist => 40202,
            Self::Reverse => 40213,
            Self::FileInfo => 40208,
            Self::BookmarkSelection => 40319,
            Self::CloseEditor => 40224,
            Self::ToggleEditorShade => 40064,
        }
    }
}

/// One row of the playlist editor's shortcut table (S03 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistBinding {
    /// The key.
    pub key: Key,
    /// The exact modifier set (no other modifier may be held).
    pub modifiers: Modifiers,
    /// The action the row triggers.
    pub action: PlaylistKeyAction,
    /// `true` when the row is provisional (S03 §2).
    pub provisional: bool,
}

const fn prow(
    key: Key,
    modifiers: Modifiers,
    action: PlaylistKeyAction,
    provisional: bool,
) -> PlaylistBinding {
    PlaylistBinding {
        key,
        modifiers,
        action,
        provisional,
    }
}

/// The playlist editor's shortcut table (S03 §2), in the spec's row order.
///
/// The 83 rows whose command this build can act on. The five remaining rows
/// of S03 §2 have no command yet and are listed in
/// [`UNIMPLEMENTED_PLAYLIST_ROWS`].
///
/// The table is the playlist window's **own** table: it shadows the main
/// window's table (S03 §1) and the global one (S03 §4) wherever the two
/// disagree — `Ctrl+A` selects all here and toggles "always on top" in the
/// main window, `Ctrl+L` adds a location here and opens one there, `Ctrl+O`
/// opens a playlist here and a minibrowser command globally. Which table
/// wins where two of them bind the same key is **provisional** (S03 §6.1);
/// this implementation gives the window's own table the last word.
///
/// A row is provisional when S03 §2 says so, and also when it repeats a
/// main-table row whose *binding* was not re-measured (the volume rows, the
/// same reason the main table marks `Up`, `Down`, `Num2` and `Num8`).
pub const PLAYLIST_TABLE: &[PlaylistBinding] = &[
    prow(
        Key::Digit(1),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::SortByTitle,
        false,
    ),
    prow(
        Key::Digit(2),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::SortByFileName,
        false,
    ),
    prow(
        Key::Digit(3),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::SortByPathAndFileName,
        false,
    ),
    prow(
        Key::Digit(3),
        Modifiers::ALT,
        PlaylistKeyAction::FileInfo,
        false,
    ),
    prow(
        Key::Letter(b'A'),
        Modifiers::CTRL,
        PlaylistKeyAction::SelectAll,
        false,
    ),
    prow(
        Key::Letter(b'A'),
        Modifiers::CTRL.union(Modifiers::ALT),
        PlaylistKeyAction::Shared(ShortcutAction::ToggleAlwaysOnTop),
        false,
    ),
    prow(
        Key::Letter(b'B'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::NextTrack),
        false,
    ),
    prow(
        Key::Letter(b'C'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::PauseUnpause),
        false,
    ),
    prow(
        Key::Letter(b'D'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleDoubleSize),
        false,
    ),
    prow(
        Key::Letter(b'E'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::TogglePlaylistEditor),
        false,
    ),
    prow(
        Key::Letter(b'F'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::OpenMainMenu),
        true,
    ),
    prow(
        Key::Letter(b'G'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleEqualizer),
        false,
    ),
    prow(
        Key::Letter(b'I'),
        Modifiers::CTRL,
        PlaylistKeyAction::InvertSelection,
        false,
    ),
    prow(
        Key::Letter(b'I'),
        Modifiers::ALT,
        PlaylistKeyAction::BookmarkSelection,
        false,
    ),
    prow(
        Key::Letter(b'J'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::JumpToFile),
        false,
    ),
    prow(
        Key::Letter(b'J'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::JumpToTime),
        false,
    ),
    prow(
        Key::Letter(b'K'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::SelectVisPlugin),
        false,
    ),
    prow(
        Key::Letter(b'K'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ConfigureVisPlugin),
        false,
    ),
    prow(
        Key::Letter(b'K'),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::Shared(ShortcutAction::ToggleVisPlugin),
        false,
    ),
    prow(
        Key::Letter(b'L'),
        Modifiers::NONE,
        PlaylistKeyAction::AddFiles,
        false,
    ),
    prow(
        Key::Letter(b'L'),
        Modifiers::CTRL,
        PlaylistKeyAction::AddLocation,
        false,
    ),
    prow(
        Key::Letter(b'L'),
        Modifiers::SHIFT,
        PlaylistKeyAction::AddDirectory,
        false,
    ),
    prow(
        Key::Letter(b'M'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::Minimize),
        true,
    ),
    prow(
        Key::Letter(b'N'),
        Modifiers::CTRL,
        PlaylistKeyAction::ClearPlaylist,
        false,
    ),
    prow(
        Key::Letter(b'N'),
        Modifiers::CTRL.union(Modifiers::ALT),
        PlaylistKeyAction::Shared(ShortcutAction::NewInstance),
        true,
    ),
    prow(
        Key::Letter(b'O'),
        Modifiers::CTRL,
        PlaylistKeyAction::OpenPlaylist,
        false,
    ),
    prow(
        Key::Letter(b'O'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::VisOptions),
        false,
    ),
    prow(
        Key::Letter(b'P'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::Preferences),
        false,
    ),
    prow(
        Key::Letter(b'R'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleRepeat),
        false,
    ),
    prow(
        Key::Letter(b'R'),
        Modifiers::CTRL,
        PlaylistKeyAction::Reverse,
        false,
    ),
    prow(
        Key::Letter(b'R'),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::Randomize,
        false,
    ),
    prow(
        Key::Letter(b'S'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleShuffle),
        false,
    ),
    prow(
        Key::Letter(b'S'),
        Modifiers::CTRL,
        PlaylistKeyAction::SavePlaylist,
        false,
    ),
    prow(
        Key::Letter(b'S'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::SkinBrowser),
        false,
    ),
    prow(
        Key::Letter(b'T'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleTimeRemaining),
        false,
    ),
    prow(
        Key::Letter(b'T'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleMinibrowser),
        false,
    ),
    prow(
        Key::Letter(b'V'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Stop),
        false,
    ),
    prow(
        Key::Letter(b'V'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::StopAfterCurrent),
        false,
    ),
    prow(
        Key::Letter(b'V'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleVideoWindow),
        false,
    ),
    prow(
        Key::Letter(b'V'),
        Modifiers::SHIFT,
        PlaylistKeyAction::Shared(ShortcutAction::StopWithFadeout),
        false,
    ),
    prow(
        Key::NumDecimal,
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::JumpToFile),
        false,
    ),
    prow(
        Key::Delete,
        Modifiers::NONE,
        PlaylistKeyAction::Delete,
        false,
    ),
    prow(Key::Delete, Modifiers::CTRL, PlaylistKeyAction::Crop, false),
    prow(
        Key::Delete,
        Modifiers::ALT,
        PlaylistKeyAction::RemoveDead,
        false,
    ),
    prow(
        Key::Delete,
        Modifiers::CTRL.union(Modifiers::SHIFT),
        PlaylistKeyAction::ClearPlaylist,
        false,
    ),
    prow(
        Key::Down,
        Modifiers::NONE,
        PlaylistKeyAction::MoveSelectionDown,
        true,
    ),
    prow(
        Key::Down,
        Modifiers::ALT,
        PlaylistKeyAction::MoveBlockDown,
        true,
    ),
    prow(
        Key::Up,
        Modifiers::NONE,
        PlaylistKeyAction::MoveSelectionUp,
        true,
    ),
    prow(
        Key::Up,
        Modifiers::ALT,
        PlaylistKeyAction::MoveBlockUp,
        true,
    ),
    prow(
        Key::Home,
        Modifiers::NONE,
        PlaylistKeyAction::JumpToFirst,
        true,
    ),
    prow(
        Key::End,
        Modifiers::NONE,
        PlaylistKeyAction::JumpToLast,
        true,
    ),
    prow(
        Key::PageUp,
        Modifiers::NONE,
        PlaylistKeyAction::PageUp,
        true,
    ),
    prow(
        Key::PageDown,
        Modifiers::NONE,
        PlaylistKeyAction::PageDown,
        true,
    ),
    prow(
        Key::Enter,
        Modifiers::NONE,
        PlaylistKeyAction::PlaySelected,
        false,
    ),
    prow(
        Key::Insert,
        Modifiers::NONE,
        PlaylistKeyAction::AddDirectory,
        false,
    ),
    prow(
        Key::Insert,
        Modifiers::CTRL,
        PlaylistKeyAction::AddFiles,
        false,
    ),
    prow(
        Key::Insert,
        Modifiers::SHIFT,
        PlaylistKeyAction::AddLocation,
        false,
    ),
    prow(
        Key::F(1),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::About),
        false,
    ),
    prow(
        Key::F(3),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::PlaylistSearch),
        true,
    ),
    prow(
        Key::F(4),
        Modifiers::CTRL,
        PlaylistKeyAction::CloseEditor,
        true,
    ),
    prow(
        Key::F(5),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::RefreshSkin),
        true,
    ),
    prow(
        Key::Left,
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Rewind5s),
        false,
    ),
    prow(
        Key::Right,
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Forward5s),
        false,
    ),
    prow(
        Key::Numpad(0),
        Modifiers::NONE,
        PlaylistKeyAction::AddFiles,
        false,
    ),
    prow(
        Key::Numpad(0),
        Modifiers::CTRL,
        PlaylistKeyAction::AddLocation,
        false,
    ),
    prow(
        Key::Numpad(1),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::TenTracksBack),
        false,
    ),
    prow(
        Key::Numpad(2),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::VolumeDown),
        true,
    ),
    prow(
        Key::Numpad(3),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::TenTracksForward),
        false,
    ),
    prow(
        Key::Numpad(4),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::PreviousTrack),
        false,
    ),
    prow(
        Key::Numpad(5),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Play),
        false,
    ),
    prow(
        Key::Numpad(6),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::NextTrack),
        false,
    ),
    prow(
        Key::Numpad(7),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Rewind5s),
        false,
    ),
    prow(
        Key::Numpad(8),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::VolumeUp),
        true,
    ),
    prow(
        Key::Numpad(9),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Forward5s),
        false,
    ),
    prow(
        Key::Enter,
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ToggleVideoFullscreen),
        true,
    ),
    prow(
        Key::Tab,
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::NextWindow),
        true,
    ),
    prow(
        Key::Letter(b'W'),
        Modifiers::CTRL,
        PlaylistKeyAction::ToggleEditorShade,
        false,
    ),
    prow(
        Key::Letter(b'W'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::ShowMainWindow),
        false,
    ),
    prow(
        Key::Letter(b'X'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::Play),
        false,
    ),
    prow(
        Key::Letter(b'X'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::OpenLocation),
        false,
    ),
    prow(
        Key::Letter(b'X'),
        Modifiers::SHIFT,
        PlaylistKeyAction::Shared(ShortcutAction::OpenFiles),
        false,
    ),
    prow(
        Key::Letter(b'Z'),
        Modifiers::NONE,
        PlaylistKeyAction::Shared(ShortcutAction::PreviousTrack),
        false,
    ),
    prow(
        Key::Letter(b'Z'),
        Modifiers::CTRL,
        PlaylistKeyAction::Shared(ShortcutAction::StartOfList),
        false,
    ),
];

/// One S03 §2 row whose command this build does not implement yet.
///
/// The row keeps its S03 command id, but no [`Command`](rustamp_core::command::Command)
/// exists for it, so the app records a note and does nothing. The rows are
/// listed here (and not in [`PLAYLIST_TABLE`]) so the table stays a table of
/// actions and the gap stays visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnimplementedPlaylistRow {
    /// The key.
    pub key: Key,
    /// The exact modifier set (no other modifier may be held).
    pub modifiers: Modifiers,
    /// The public `WM_COMMAND` id S03 §2 records for the row.
    pub command_id: u32,
    /// `true` when the row is provisional (S03 §2).
    pub provisional: bool,
}

/// The S03 §2 rows this build cannot act on yet.
///
/// * `Ctrl+E` — `40255` "Edit the playlist entry": no command edits a
///   playlist entry, and the target of the edit (the selected entry, not the
///   current track) is not specified either.
/// * `Ctrl+Alt+E` — `40293` "Read extended info on selection": no command
///   reads extended info.
/// * `Ctrl+Alt+G` — `40292` "Generate HTML playlist": no command writes an
///   HTML playlist.
/// * `Ctrl+Num+` / `Ctrl+Num-` — `40335` / `40336` "Increase/Decrease the
///   playlist font size": the playlist font comes from the skin's
///   `pledit.txt` and nothing changes it at runtime.
pub const UNIMPLEMENTED_PLAYLIST_ROWS: &[UnimplementedPlaylistRow] = &[
    UnimplementedPlaylistRow {
        key: Key::Letter(b'E'),
        modifiers: Modifiers::CTRL,
        command_id: 40255,
        provisional: false,
    },
    UnimplementedPlaylistRow {
        key: Key::Letter(b'E'),
        modifiers: Modifiers::CTRL.union(Modifiers::ALT),
        command_id: 40293,
        provisional: false,
    },
    UnimplementedPlaylistRow {
        key: Key::Letter(b'G'),
        modifiers: Modifiers::CTRL.union(Modifiers::ALT),
        command_id: 40292,
        provisional: false,
    },
    UnimplementedPlaylistRow {
        key: Key::NumAdd,
        modifiers: Modifiers::CTRL,
        command_id: 40335,
        provisional: true,
    },
    UnimplementedPlaylistRow {
        key: Key::NumSubtract,
        modifiers: Modifiers::CTRL,
        command_id: 40336,
        provisional: true,
    },
];

/// The S03 §2 row a key press names, if the table binds it.
///
/// The lookup is exact: a row binds one modifier set and no other modifier
/// may be held. Letters are matched case-insensitively.
pub fn playlist_key_binding(key: Key, modifiers: Modifiers) -> Option<&'static PlaylistBinding> {
    let key = match key {
        Key::Letter(c) => Key::Letter(c.to_ascii_uppercase()),
        other => other,
    };
    PLAYLIST_TABLE
        .iter()
        .find(|binding| binding.key == key && binding.modifiers == modifiers)
}

/// The S03 §2 row a key press names but this build cannot act on, if any.
pub fn unimplemented_playlist_row(
    key: Key,
    modifiers: Modifiers,
) -> Option<&'static UnimplementedPlaylistRow> {
    let key = match key {
        Key::Letter(c) => Key::Letter(c.to_ascii_uppercase()),
        other => other,
    };
    UNIMPLEMENTED_PLAYLIST_ROWS
        .iter()
        .find(|row| row.key == key && row.modifiers == modifiers)
}

/// Maps a key press to the playlist editor's key action, if any.
///
/// The map is [`PLAYLIST_TABLE`]: every S03 §2 row this build can act on, in
/// the spec's order. The rows S03 §2 binds but this build cannot act on are
/// not actions — they are listed in [`UNIMPLEMENTED_PLAYLIST_ROWS`] and
/// resolve through [`unimplemented_playlist_row`] instead.
///
/// The app consults the map in `App::playlist_key_down`; this function only
/// names the row, it never runs it.
pub fn playlist_key_action(key: Key, modifiers: Modifiers) -> Option<PlaylistKeyAction> {
    playlist_key_binding(key, modifiers).map(|binding| binding.action)
}

/// A dialog the playlist window asks the app to open (S06 §4).
///
/// The spec settles existence and titles (from window metadata) and fixes no
/// dialog behaviour beyond that, so these carry the title only: dialog
/// internals are intentionally not modelled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaylistDialog {
    /// `Add file(s) to playlist` (`#32770`, 1503 x 778).
    AddFiles,
    /// `Load Playlist` (`#32770`, 1503 x 778).
    LoadPlaylist,
}

impl PlaylistDialog {
    /// The dialog's window title (S06 §4, settled from window metadata).
    pub const fn title(self) -> &'static str {
        match self {
            PlaylistDialog::AddFiles => ADD_DIALOG_TITLE,
            PlaylistDialog::LoadPlaylist => LOAD_DIALOG_TITLE,
        }
    }

    /// The core command that opens this dialog, when one exists.
    ///
    /// ADD maps to [`Command::OpenMedia`] with [`OpenMediaKind::Files`]; the
    /// `play` value is unmeasured for the playlist-window add path (RFC 0005
    /// O5), so this uses `false` and the finding is reported by the caller.
    /// LIST OPTS maps to [`Command::OpenPlaylist`] (RFC 0010): the spec fixes
    /// the dialog's title only, so the app records the request and implements
    /// no dialog behaviour beyond it.
    pub const fn opening_command(self) -> Option<Command> {
        match self {
            PlaylistDialog::AddFiles => Some(Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            }),
            PlaylistDialog::LoadPlaylist => Some(Command::OpenPlaylist),
        }
    }
}

/// Title of the ADD dialog (S06 §4, settled from window metadata).
pub const ADD_DIALOG_TITLE: &str = "Add file(s) to playlist";
/// Title of the LIST OPTS dialog (S06 §4, settled from window metadata).
pub const LOAD_DIALOG_TITLE: &str = "Load Playlist";

/// Whether the playlist title bar carries a window-menu button.
///
/// S06 §1 (settled with qualification, one R1 timing): a posted click at
/// (6,3) delivered no `#32768` popup, so there is no window-menu button and
/// nothing in the UI may offer one here. Kept as a function (rather than a
/// bare constant) so call sites read as the spec rule.
pub const fn has_window_menu_button() -> bool {
    false
}

/// The ids a shift-click from `anchor` to `target` selects, over a bare entry list.
///
/// Replicates the clamped range rule from `rustamp_playlist::range_ids` so that
/// `rustamp-ui` stays dependency-free of `rustamp-playlist`.
pub fn range_ids(entries: &[EntryId], anchor: usize, target: usize) -> Vec<EntryId> {
    if entries.is_empty() {
        return Vec::new();
    }
    let a = anchor.min(entries.len() - 1);
    let b = target.min(entries.len() - 1);
    let (first, last) = if a <= b { (a, b) } else { (b, a) };
    entries[first..=last].to_vec()
}

/// The playlist editor's headless behaviour: list view, selection and the
/// S06-settled gestures.
///
/// The machine tracks entry order as [`EntryId`]s (synced from the app via
/// [`PlaylistWindow::set_entries`]); selection is a set of ids. The visible
/// slice is [`VISIBLE_ROWS`] entries starting at the scroll offset.
#[derive(Debug, Clone, Default)]
pub struct PlaylistWindow {
    /// Render the skin's 275x14 playlist windowshade band (S01 §1/§4).
    pub windowshade: bool,
    /// Held title button, overlaid from pledit.bmp; bottom buttons have no
    /// pressed sprite (S01 §4.1).
    pub pressed_title_button: Option<crate::playlist_window::PlaylistTitleButton>,
    entries: Vec<EntryId>,
    first_visible: usize,
    selected: BTreeSet<EntryId>,
    shift_anchor: Option<EntryId>,
    current: Option<EntryId>,
    drag: Option<PlaylistDrag>,
}

impl PlaylistWindow {
    /// An empty playlist window showing nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Syncs the entry order (the app is the state owner).
    ///
    /// Entries that no longer exist drop out of the selection; the scroll
    /// offset is clamped so the visible slice stays inside the list.
    pub fn set_entries(&mut self, entries: Vec<EntryId>) {
        if self.entries != entries {
            self.cancel_drag();
        }
        self.entries = entries;
        let live: BTreeSet<EntryId> = self.entries.iter().copied().collect();
        self.selected.retain(|id| live.contains(id));
        if let Some(anchor) = self.shift_anchor
            && !live.contains(&anchor)
        {
            self.shift_anchor = None;
        }
        if let Some(cur) = self.current
            && !live.contains(&cur)
        {
            self.current = None;
        }
        self.clamp_scroll();
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The current playback entry, independently of the selection and viewport.
    pub fn current(&self) -> Option<EntryId> {
        self.current
    }

    /// Syncs the current playback entry from the app without selecting or scrolling.
    ///
    /// Unknown ids clear the marker. Like [`Self::set_entries`], this only
    /// mirrors app-owned state; it never predicts playback from a click.
    pub fn set_current(&mut self, current: Option<EntryId>) {
        self.current = current.filter(|id| self.entries.contains(id));
    }

    /// Index of the first visible entry (the scroll offset).
    pub fn first_visible(&self) -> usize {
        self.first_visible
    }

    /// The currently visible entries, at most [`VISIBLE_ROWS`] (S06 §3).
    pub fn visible_ids(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .skip(self.first_visible)
            .take(VISIBLE_ROWS)
            .copied()
            .collect()
    }

    /// The selected entries, in playlist order.
    pub fn selected_ids(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .copied()
            .filter(|id| self.selected.contains(id))
            .collect()
    }

    /// Presses a row and arms a drag, preserving an existing multi-selection
    /// when the pressed row belongs to it. Returns a selection command only
    /// when pressing an unselected row. Own choice, not settled by S06 §8 Q6.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Option<Command> {
        self.cancel_drag();
        if self.windowshade {
            return None;
        }
        let row = crate::playlist_window::hit_test_list_row(x, y)?;
        let id = *self.entries.get(self.first_visible + row)?;
        let command = if self.selected.contains(&id) {
            None
        } else {
            self.selected.clear();
            self.selected.insert(id);
            self.shift_anchor = Some(id);
            Some(Command::Select(SelectOp::Set(vec![id])))
        };
        self.drag = Some(PlaylistDrag {
            press: (x, y),
            ids: self.selected_ids(),
            insertion: None,
        });
        command
    }

    /// Updates an armed drag. Four pixels of motion on either axis activate
    /// it; leaving the list cancels instead of clamping or auto-scrolling.
    /// Upper/lower row halves mean insert-before/after (own choices, Q6).
    pub fn mouse_move(&mut self, x: i32, y: i32) {
        use crate::playlist_window::{LIST_TOP, ROW_PITCH, in_list_area};

        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if self.windowshade || !in_list_area(x, y) {
            self.cancel_drag();
            return;
        }
        if drag.insertion.is_none()
            && x.abs_diff(drag.press.0).max(y.abs_diff(drag.press.1)) < DRAG_THRESHOLD
        {
            return;
        }
        let relative = y - LIST_TOP;
        let row = relative / ROW_PITCH;
        let after = relative % ROW_PITCH >= (ROW_PITCH + 1) / 2;
        let gap = (row as usize + usize::from(after)).min(VISIBLE_ROWS);
        drag.insertion = Some((self.first_visible + gap).min(self.entries.len()));
    }

    /// Releases a drag as the existing `MoveEntries` command. Its target is
    /// measured *after* removing the moved ids. A click or unchanged order
    /// emits nothing. The app alone applies the move; this model never predicts
    /// a new playing row or changes the order before the command is accepted.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> Option<Command> {
        self.mouse_move(x, y);
        let drag = self.drag.take()?;
        let gap = drag.insertion?;
        let moved: BTreeSet<_> = drag.ids.iter().copied().collect();
        let positions: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, id)| moved.contains(id).then_some(index))
            .collect();
        let to = gap - positions.iter().filter(|&&index| index < gap).count();
        if positions.iter().copied().eq(to..to + positions.len()) {
            return None;
        }
        Some(Command::MoveEntries {
            ids: drag.ids,
            to: u32::try_from(to).ok()?,
        })
    }

    /// The active insertion gap in playlist coordinates, before removing the
    /// dragged rows. `None` until the threshold is crossed or after cancellation.
    pub fn drag_insertion(&self) -> Option<usize> {
        self.drag.as_ref().and_then(|drag| drag.insertion)
    }

    /// Cancels a pending/active drag without moving entries or clearing selection.
    /// Returns whether a gesture was cancelled (including a press below slop).
    pub fn cancel_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// A double-click on visible row `row` plays that entry (S03 §5, S06 §8 Q5).
    ///
    /// Selects the entry and returns [`Command::PlayEntry`] with the entry's stable id.
    pub fn double_click_row(&mut self, row: usize) -> Option<Command> {
        let id = self.visible_ids().get(row).copied()?;
        self.selected.clear();
        self.selected.insert(id);
        Some(Command::PlayEntry(id))
    }

    /// A double-click on entry `index` in the list (0-based) plays that entry.
    pub fn double_click_index(&mut self, index: usize) -> Option<Command> {
        let id = self.entries.get(index).copied()?;
        self.selected.clear();
        self.selected.insert(id);
        Some(Command::PlayEntry(id))
    }

    /// Applies a selection operation to the window's selection set.
    pub fn apply_select(&mut self, op: &SelectOp) {
        match op {
            SelectOp::Set(ids) => {
                let live: BTreeSet<EntryId> = self.entries.iter().copied().collect();
                self.selected = ids.iter().copied().filter(|id| live.contains(id)).collect();
            }
            SelectOp::Toggle(id) => {
                if self.entries.contains(id) && !self.selected.remove(id) {
                    self.selected.insert(*id);
                }
            }
            SelectOp::All => {
                self.selected = self.entries.iter().copied().collect();
            }
            SelectOp::None => {
                self.selected.clear();
            }
            SelectOp::Invert => {
                let mut inverted = BTreeSet::new();
                for id in &self.entries {
                    if !self.selected.contains(id) {
                        inverted.insert(*id);
                    }
                }
                self.selected = inverted;
            }
        }
        if self.drag.as_ref().is_some_and(|drag| {
            drag.ids.len() != self.selected.len()
                || drag.ids.iter().any(|id| !self.selected.contains(id))
        }) {
            self.cancel_drag();
        }
    }

    /// SEL selects all visible rows (S06 §2, settled: click2-vs-click1 12203
    /// px, bbox (12,23)-(254,93); "all" is a visual/blue-band inference
    /// consistent with the counts).
    pub fn press_sel(&mut self) -> Command {
        let visible = self.visible_ids();
        self.selected = visible.iter().copied().collect();
        Command::Select(SelectOp::Set(visible))
    }

    /// Ctrl+A selects every entry in the list (S03 §2, `40205` "Select all",
    /// menu-confirmed). The spec states the binding, not a measured effect, so
    /// this selects the whole list rather than the visible page that SEL
    /// selects (S06 §2).
    ///
    /// It only *produces* the command: the app applies `Command::Select` to
    /// this model, so applying it here as well would apply it twice — which is
    /// harmless for `All` but not for [`Self::invert_selection`].
    pub fn select_all(&mut self) -> Command {
        Command::Select(SelectOp::All)
    }

    /// Ctrl+I inverts the selection (S03 §2, `40171` "Invert selection",
    /// menu-confirmed; effect not measured). Like [`Self::select_all`] it only
    /// produces the command, because inverting twice is the identity.
    pub fn invert_selection(&mut self) -> Command {
        Command::Select(SelectOp::Invert)
    }

    /// Moves the selection one entry up (S03 §2, `40289` "Move the selection
    /// up", **provisional** with no measured effect).
    ///
    /// Own choice, stated plainly: the model keeps a selection set, not a
    /// cursor, so this collapses to a single selection one entry above the
    /// current first-selected entry (the playlist-order anchor). With no
    /// selection it selects the last visible row. The view follows so the new
    /// selection stays visible. A no-op on an empty list.
    pub fn move_selection_up(&mut self) {
        let Some(index) = self.selection_anchor_index(false) else {
            return;
        };
        let index = index.saturating_sub(1);
        self.select_single_and_reveal(index);
    }

    /// Moves the selection one entry down (S03 §2, `40290` "Move the selection
    /// down", **provisional** with no measured effect).
    ///
    /// Own choice, stated plainly: mirrors [`Self::move_selection_up`]; with
    /// no selection it selects the first visible row.
    pub fn move_selection_down(&mut self) {
        let Some(index) = self.selection_anchor_index(true) else {
            return;
        };
        let index = (index + 1).min(self.entries.len().saturating_sub(1));
        self.select_single_and_reveal(index);
    }

    /// Moves the selected block one entry up (S03 §2, `40215` "Move the
    /// selected entry up", **provisional** with no measured effect).
    ///
    /// Own choice, stated plainly: the selected entries keep their relative
    /// order and move one row up as a block, which is the existing
    /// [`Command::MoveEntries`] contract. The block is the selection itself,
    /// so a selection that is not contiguous moves as one block anyway (a
    /// choice, not a measurement). `None` when nothing is selected or when
    /// the block already starts the list.
    ///
    /// The command is only produced: the app applies it and re-syncs the
    /// model, like every other order change.
    pub fn move_block_up(&self) -> Option<Command> {
        self.move_block(true)
    }

    /// Moves the selected block one entry down (S03 §2, `40216` "Move the
    /// selected entry down", **provisional** with no measured effect).
    ///
    /// Own choice: mirrors [`Self::move_block_up`]. `None` when nothing is
    /// selected or when the block already ends the list.
    pub fn move_block_down(&self) -> Option<Command> {
        self.move_block(false)
    }

    /// The [`Command::MoveEntries`] that moves the selection one row up or
    /// down, or `None` when the move is impossible or a no-op.
    ///
    /// The target index is the row the *first* moved entry lands at in the
    /// list without the moved entries — the same contract
    /// [`Command::MoveEntries`] states. Moving up therefore lands on
    /// `first - 1` and moving down on `first + 1`.
    fn move_block(&self, up: bool) -> Option<Command> {
        let ids = self.selected_ids();
        let first = self.entries.iter().position(|entry| ids.contains(entry))?;
        let last = self.entries.iter().rposition(|entry| ids.contains(entry))?;
        let target = if up {
            if first == 0 {
                return None;
            }
            first - 1
        } else {
            if last + 1 >= self.entries.len() {
                return None;
            }
            first + 1
        };
        Some(Command::MoveEntries {
            ids,
            to: u32::try_from(target).ok()?,
        })
    }

    /// Jumps to the first entry (S03 §2, `40264` "Jump to the first entry",
    /// **provisional** with no measured effect).
    ///
    /// Own choice: selects the first entry alone and scrolls to the top.
    /// A no-op on an empty list.
    pub fn jump_to_first(&mut self) {
        if !self.entries.is_empty() {
            self.select_single_and_reveal(0);
        }
    }

    /// Jumps to the last entry (S03 §2, `40265` "Jump to the last entry",
    /// **provisional** with no measured effect).
    ///
    /// Own choice: selects the last entry alone and scrolls so it is visible.
    /// A no-op on an empty list.
    pub fn jump_to_last(&mut self) {
        if let Some(last) = self.entries.len().checked_sub(1) {
            self.select_single_and_reveal(last);
        }
    }

    /// Plays the selected entry (S03 §2, `40184` "Play the selected item(s)",
    /// menu-confirmed; the effect is not measured).
    ///
    /// Own choice, stated plainly: the spec does not say which entry plays
    /// when several are selected, so this plays the first selected entry in
    /// playlist order and selects it alone. `None` when nothing is selected.
    pub fn play_selected(&mut self) -> Option<Command> {
        let id = self.selected_ids().into_iter().next()?;
        self.selected.clear();
        self.selected.insert(id);
        Some(Command::PlayEntry(id))
    }

    /// The anchor for [`Self::move_selection_up`] and
    /// [`Self::move_selection_down`]: the index of the first selected entry
    /// in playlist order, or — with no selection — the first visible row
    /// (`down == true`) or the last visible row (`down == false`).
    fn selection_anchor_index(&self, down: bool) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        if let Some(id) = self.selected_ids().into_iter().next()
            && let Some(index) = self.entries.iter().position(|entry| *entry == id)
        {
            return Some(index);
        }
        let visible = self.visible_ids();
        if visible.is_empty() {
            return None;
        }
        let id = if down {
            visible[0]
        } else {
            visible[visible.len() - 1]
        };
        self.entries.iter().position(|entry| *entry == id)
    }

    /// Selects `entries[index]` alone and scrolls so it is visible.
    fn select_single_and_reveal(&mut self, index: usize) {
        let Some(id) = self.entries.get(index).copied() else {
            return;
        };
        self.selected.clear();
        self.selected.insert(id);
        if index < self.first_visible {
            self.scroll_to(index);
        } else if index >= self.first_visible + VISIBLE_ROWS {
            self.scroll_to(index + 1 - VISIBLE_ROWS);
        }
    }

    /// REM removes the selected entries (S06 §5, settled on both the real and
    /// the posted path: 3475 px, same bounds (12,23)-(254,93)).
    ///
    /// The removal is a normal [`Command::Remove`]: no `#32768` window is
    /// present at capture time and no menu choice is required (S06 §5.2). With
    /// nothing selected the button is a no-op returning `None` (S06 §5.3: R2's
    /// unstaged REM second click changed 0 px).
    pub fn press_rem(&mut self) -> Option<Command> {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return None;
        }
        let gone: BTreeSet<EntryId> = ids.iter().copied().collect();
        self.entries.retain(|id| !gone.contains(id));
        self.selected.clear();
        self.clamp_scroll();
        Some(Command::Remove(ids))
    }

    /// ADD opens the add-files dialog (S06 §4); the spec fixes no dialog
    /// behaviour beyond the title, so this emits the opening command only.
    pub fn press_add(&self) -> Command {
        // `PlaylistDialog::AddFiles.opening_command` is `Some` by
        // construction; the expect names the invariant.
        PlaylistDialog::AddFiles
            .opening_command()
            .expect("ADD has an opening command (S06 §4)")
    }

    /// LIST OPTS opens the load-playlist dialog (S06 §4); the spec fixes no
    /// dialog behaviour beyond the title, so this returns the dialog request
    /// for the app to open. The request's opening command is
    /// [`Command::OpenPlaylist`] (RFC 0010).
    pub fn press_list_opts(&self) -> PlaylistDialog {
        PlaylistDialog::LoadPlaylist
    }

    /// One scrollbar-arrow step scrolls one page of four rows (S06 §3,
    /// settled as a pixel symmetry: 1501 px each way, returning on the
    /// reverse arrow). Clamped at both ends of the list.
    pub fn scroll_page(&mut self, direction: ScrollDirection) {
        let max_first = self.max_first_visible();
        match direction {
            ScrollDirection::Up => {
                self.first_visible = self.first_visible.saturating_sub(PAGE_STEP_ROWS);
            }
            ScrollDirection::Down => {
                self.first_visible = (self.first_visible + PAGE_STEP_ROWS).min(max_first);
            }
        }
    }

    /// Scrolls so `index` is the first visible entry, clamped into the list.
    pub fn scroll_to(&mut self, index: usize) {
        self.first_visible = index.min(self.max_first_visible());
    }

    /// The largest scroll offset: the bottom page shows the last
    /// [`VISIBLE_ROWS`] entries. S01 §4.2 measured the bottom view of the
    /// 60-entry list as rows 57-60 with the handle at its lowest point, so
    /// the offset range is `len - 4`, the same range
    /// [`scroll_handle_y`](crate::scroll_handle_y) assumes. A list that fits
    /// one page has offset 0.
    pub fn max_first_visible(&self) -> usize {
        self.entries.len().saturating_sub(VISIBLE_ROWS)
    }

    fn clamp_scroll(&mut self) {
        self.first_visible = self.first_visible.min(self.max_first_visible());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: u64) -> Vec<EntryId> {
        (0..n).map(EntryId).collect()
    }

    fn window_with(n: u64) -> PlaylistWindow {
        let mut window = PlaylistWindow::new();
        window.set_entries(ids(n));
        window
    }

    #[test]
    fn row_drag_crosses_slop_then_chooses_row_halves() {
        let mut window = window_with(4);
        assert_eq!(
            window.mouse_down(60, 28),
            Some(Command::Select(SelectOp::Set(vec![EntryId(0)])))
        );
        window.mouse_move(63, 28);
        assert_eq!(window.drag_insertion(), None);
        window.mouse_move(64, 28);
        assert_eq!(window.drag_insertion(), Some(0));
        window.mouse_move(60, 55); // Upper half of row 2.
        assert_eq!(window.drag_insertion(), Some(2));
        window.mouse_move(60, 56); // Lower half of row 2.
        assert_eq!(window.drag_insertion(), Some(3));
        assert_eq!(
            window.mouse_up(60, 56),
            Some(Command::MoveEntries {
                ids: vec![EntryId(0)],
                to: 2,
            })
        );
        assert_eq!(window.drag_insertion(), None);
        assert_eq!(window.visible_ids(), ids(4), "the app owns order");
    }

    #[test]
    fn dragging_a_selected_row_keeps_all_selected_ids_in_playlist_order() {
        let mut window = window_with(8);
        let selection = vec![EntryId(1), EntryId(6)];
        window.apply_select(&SelectOp::Set(vec![EntryId(6), EntryId(1)]));
        window.set_current(Some(EntryId(6)));
        assert_eq!(window.mouse_down(60, 42), None);
        window.set_entries(ids(8)); // A same-order app sync must not cancel.
        window.mouse_move(60, 23);
        assert_eq!(
            window.mouse_up(60, 23),
            Some(Command::MoveEntries {
                ids: selection.clone(),
                to: 0,
            })
        );
        window.set_entries(vec![
            EntryId(1),
            EntryId(6),
            EntryId(0),
            EntryId(2),
            EntryId(3),
            EntryId(4),
            EntryId(5),
            EntryId(7),
        ]);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(1), EntryId(6), EntryId(0), EntryId(2)]
        );
        assert_eq!(window.selected_ids(), selection);
        assert_eq!(window.current(), Some(EntryId(6)));
    }

    #[test]
    fn row_drag_uses_scrolled_gaps_and_blank_space_after_the_last_entry() {
        let mut window = window_with(8);
        window.scroll_to(4);
        window.mouse_down(60, 28);
        window.mouse_move(60, 74);
        assert_eq!(window.drag_insertion(), Some(8));
        assert_eq!(
            window.mouse_up(60, 74),
            Some(Command::MoveEntries {
                ids: vec![EntryId(4)],
                to: 7,
            })
        );

        let mut window = window_with(2);
        assert_eq!(
            window.mouse_down(60, 70),
            None,
            "blank space cannot start a drag"
        );
        window.mouse_down(60, 28);
        window.mouse_move(60, 85);
        assert_eq!(window.drag_insertion(), Some(2));
        assert_eq!(
            window.mouse_up(60, 85),
            Some(Command::MoveEntries {
                ids: vec![EntryId(0)],
                to: 1,
            })
        );
    }

    #[test]
    fn clicks_and_drops_around_an_unchanged_block_emit_no_move() {
        let mut window = window_with(4);
        window.mouse_down(60, 28);
        assert_eq!(window.mouse_up(63, 28), None);
        for y in [36, 48, 49, 61] {
            window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(2)]));
            window.mouse_down(60, 42);
            window.mouse_move(64, y);
            assert!(window.drag_insertion().is_some());
            assert_eq!(window.mouse_up(64, y), None, "y={y}");
            assert_eq!(window.visible_ids(), ids(4));
        }
    }

    #[test]
    fn cancelling_leaving_or_changing_entries_prevents_a_later_drop() {
        let mut window = window_with(4);
        for leave in [
            None,
            Some((11, 50)),
            Some((254, 50)),
            Some((60, 22)),
            Some((60, 93)),
        ] {
            window.mouse_down(60, 28);
            window.mouse_move(60, 74);
            assert_eq!(window.drag_insertion(), Some(4));
            if let Some((x, y)) = leave {
                window.mouse_move(x, y);
            } else {
                assert!(window.cancel_drag());
            }
            assert_eq!(window.drag_insertion(), None);
            assert_eq!(window.mouse_up(60, 74), None);
            assert_eq!(window.visible_ids(), ids(4));
            assert_eq!(window.selected_ids(), vec![EntryId(0)]);
        }
        window.mouse_down(60, 28);
        window.mouse_move(60, 74);
        window.set_entries(vec![EntryId(1), EntryId(0), EntryId(2), EntryId(3)]);
        assert_eq!(window.mouse_up(60, 74), None, "concurrent reorder cancels");
        window.mouse_down(60, 28);
        window.mouse_move(60, 74);
        window.apply_select(&SelectOp::All);
        assert_eq!(window.mouse_up(60, 74), None, "changed selection cancels");
    }

    #[test]
    fn current_entry_is_app_owned_and_independent_of_selection_and_scroll() {
        let mut window = window_with(8);
        window.apply_select(&SelectOp::Set(vec![EntryId(5)]));
        window.scroll_to(4);
        window.set_current(Some(EntryId(1)));
        assert_eq!(window.current(), Some(EntryId(1)));
        assert_eq!(window.selected_ids(), vec![EntryId(5)]);
        assert_eq!(window.first_visible(), 4);
        window.double_click_row(2);
        assert_eq!(
            window.current(),
            Some(EntryId(1)),
            "a click is not playback"
        );
        window.set_entries(ids(8));
        assert_eq!(window.current(), Some(EntryId(1)), "survives a sync");
        window.set_entries(vec![EntryId(5)]);
        assert_eq!(
            window.current(),
            None,
            "removed entries cannot stay current"
        );
        window.set_current(Some(EntryId(99)));
        assert_eq!(window.current(), None, "unknown ids cannot become current");
        window.set_current(Some(EntryId(5)));
        window.set_current(None);
        assert_eq!(window.current(), None, "the app can clear the marker");
    }

    #[test]
    fn a_single_click_selects_the_first_visible_row() {
        // S06 §2: single posted click at (60,28) selects the first visible row.
        let mut window = window_with(60);
        window.scroll_to(56);
        let command = window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1);
        assert_eq!(
            command,
            Some(Command::Select(SelectOp::Set(vec![EntryId(56)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(56)]);
        // Row order is unchanged (S06 §2).
        assert_eq!(window.len(), 60);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(56), EntryId(57), EntryId(58), EntryId(59)]
        );
    }

    #[test]
    fn clicking_an_empty_list_selects_nothing() {
        let mut window = PlaylistWindow::new();
        assert_eq!(
            window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1),
            None
        );
        assert!(window.selected_ids().is_empty());
    }

    #[test]
    fn sel_selects_all_visible_rows() {
        // S06 §2: SEL (second real click at (74,92)) selects all visible rows.
        let mut window = window_with(60);
        window.scroll_to(56);
        let command = window.press_sel();
        let all = vec![EntryId(56), EntryId(57), EntryId(58), EntryId(59)];
        assert_eq!(command, Command::Select(SelectOp::Set(all.clone())));
        assert_eq!(window.selected_ids(), all);
    }

    #[test]
    fn sel_on_a_short_tail_selects_only_the_visible_rows() {
        let mut window = window_with(3);
        let command = window.press_sel();
        assert_eq!(
            command,
            Command::Select(SelectOp::Set(vec![EntryId(0), EntryId(1), EntryId(2)]))
        );
    }

    #[test]
    fn rem_removes_the_selected_entry_as_a_plain_command() {
        // S06 §5: REM removes the selected entry with no menu choice required.
        let mut window = window_with(60);
        window.scroll_to(56);
        window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1);
        let command = window.press_rem();
        assert_eq!(command, Some(Command::Remove(vec![EntryId(56)])));
        assert_eq!(window.len(), 59);
        assert!(window.selected_ids().is_empty());
        // The list renumbers (S06 §5.4). The bottom page of 59 entries starts
        // at offset 55, so the view shows entries 55 and 57-59.
        assert_eq!(window.first_visible(), 55);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(55), EntryId(57), EntryId(58), EntryId(59)]
        );
    }

    #[test]
    fn rem_without_a_staged_selection_is_a_no_op() {
        // S06 §5.3: without a staged selection the button is a no-op (0 px).
        let mut window = window_with(60);
        assert_eq!(window.press_rem(), None);
        assert_eq!(window.len(), 60);
    }

    #[test]
    fn rem_removes_every_selected_row() {
        let mut window = window_with(6);
        window.press_sel();
        let command = window.press_rem();
        assert_eq!(
            command,
            Some(Command::Remove(vec![
                EntryId(0),
                EntryId(1),
                EntryId(2),
                EntryId(3)
            ]))
        );
        assert_eq!(window.len(), 2);
    }

    #[test]
    fn one_arrow_step_scrolls_one_page_of_four_rows_and_returns() {
        // S06 §3: one arrow step is one page of four rows, symmetric both ways.
        let mut window = window_with(60);
        window.scroll_to(56);
        window.scroll_page(ScrollDirection::Up);
        assert_eq!(window.first_visible(), 52);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(52), EntryId(53), EntryId(54), EntryId(55)]
        );
        window.scroll_page(ScrollDirection::Down);
        assert_eq!(window.first_visible(), 56);
    }

    #[test]
    fn eight_up_steps_from_the_bottom_page_reach_rows_25_to_28() {
        // S06 §4.2 (round 3): eight up-clicks from rows 57-60 land on 25-28.
        let mut window = window_with(60);
        window.scroll_to(56);
        for _ in 0..8 {
            window.scroll_page(ScrollDirection::Up);
        }
        assert_eq!(window.first_visible(), 24);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(24), EntryId(25), EntryId(26), EntryId(27)]
        );
    }

    #[test]
    fn scrolling_clamps_at_both_ends() {
        // The bottom page is the last four entries (S01 §4.2: rows 57-60 of
        // the 60-entry list), so the offset stops at len - 4, never one row.
        let mut window = window_with(6);
        window.scroll_page(ScrollDirection::Up);
        assert_eq!(window.first_visible(), 0);
        for _ in 0..10 {
            window.scroll_page(ScrollDirection::Down);
        }
        assert_eq!(window.first_visible(), 2);
        assert_eq!(
            window.visible_ids(),
            vec![EntryId(2), EntryId(3), EntryId(4), EntryId(5)]
        );
    }

    #[test]
    fn a_list_that_fits_one_page_never_scrolls() {
        let mut window = window_with(3);
        window.scroll_page(ScrollDirection::Down);
        assert_eq!(window.first_visible(), 0);
        assert_eq!(window.visible_ids().len(), 3);
    }

    #[test]
    fn the_scroll_range_follows_adds_removals_and_a_clear() {
        // S06 §3 scope: the view stays inside the list when entries change.
        let mut window = window_with(60);
        window.scroll_to(56);
        // Shrinking to 58 entries pulls the bottom page up to offset 54.
        window.set_entries(ids(58));
        assert_eq!(window.first_visible(), 54);
        assert_eq!(window.visible_ids().len(), VISIBLE_ROWS);
        // Growing again does not move the view by itself.
        window.set_entries(ids(60));
        assert_eq!(window.first_visible(), 54);
        // A clear resets the view to the top.
        window.set_entries(Vec::new());
        assert_eq!(window.first_visible(), 0);
        assert!(window.visible_ids().is_empty());
    }

    #[test]
    fn the_bottom_page_matches_the_handle_at_its_lowest_point() {
        // S01 §4.2: handle y = 60 at the bottom of a 60-entry list (rows 57-60).
        let mut window = window_with(60);
        for _ in 0..30 {
            window.scroll_page(ScrollDirection::Down);
        }
        assert_eq!(window.first_visible(), 56);
        assert_eq!(
            crate::playlist_window::scroll_handle_y(window.first_visible(), window.len()),
            60
        );
    }

    #[test]
    fn ctrl_a_selects_every_entry_and_ctrl_i_inverts_the_selection() {
        // S03 §2: Ctrl+A (`40205`) and Ctrl+I (`40171`) are menu-confirmed rows.
        // The window only *produces* the command; the app applies it (see
        // `App::apply_app`), so the test applies it the same way - and that is
        // what keeps Invert from being applied twice.
        let mut window = window_with(6);
        assert_eq!(window.select_all(), Command::Select(SelectOp::All));
        window.apply_select(&SelectOp::All);
        assert_eq!(window.selected_ids(), ids(6));

        window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(3)]));
        assert_eq!(window.invert_selection(), Command::Select(SelectOp::Invert));
        window.apply_select(&SelectOp::Invert);
        assert_eq!(
            window.selected_ids(),
            vec![EntryId(0), EntryId(2), EntryId(4), EntryId(5)]
        );
    }

    #[test]
    fn add_opens_the_add_files_dialog_and_list_opts_the_load_dialog() {
        // S06 §4: titles from window metadata; no behaviour beyond that.
        let window = PlaylistWindow::new();
        assert_eq!(
            window.press_add(),
            Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            }
        );
        assert_eq!(window.press_list_opts(), PlaylistDialog::LoadPlaylist);
        assert_eq!(PlaylistDialog::AddFiles.title(), "Add file(s) to playlist");
        assert_eq!(PlaylistDialog::LoadPlaylist.title(), "Load Playlist");
    }

    #[test]
    fn list_opts_maps_to_the_open_playlist_command() {
        // RFC 0010: LIST OPTS opens the `Load Playlist` dialog (S06 §4), so its
        // opening command is the new app-level variant — not `None`, and not
        // the media-opening command (whose confirmation is `AddItems` with
        // append semantics the spec does not state for this dialog).
        assert_eq!(
            PlaylistDialog::LoadPlaylist.opening_command(),
            Some(Command::OpenPlaylist)
        );
        let window = PlaylistWindow::new();
        let dialog = window.press_list_opts();
        assert_eq!(dialog, PlaylistDialog::LoadPlaylist);
        assert_eq!(dialog.title(), "Load Playlist");
        assert_eq!(dialog.opening_command(), Some(Command::OpenPlaylist));
    }

    #[test]
    fn every_playlist_dialog_has_an_opening_command() {
        // Both S06 §4 dialogs are core commands now; the app opens the OS
        // window for either (RFC 0010).
        for dialog in [PlaylistDialog::AddFiles, PlaylistDialog::LoadPlaylist] {
            assert!(
                dialog.opening_command().is_some(),
                "{dialog:?} has no opening command"
            );
        }
    }

    #[test]
    fn the_settled_bottom_bar_points_hit_their_buttons() {
        // S01 §4.1/§4.2 posted positives at y = 92.
        assert_eq!(hit_test_button(24, 92), Some(PlaylistButton::Add));
        assert_eq!(hit_test_button(46, 92), Some(PlaylistButton::Rem));
        assert_eq!(hit_test_button(74, 92), Some(PlaylistButton::Sel));
        assert_eq!(hit_test_button(112, 92), Some(PlaylistButton::Misc));
        assert_eq!(hit_test_button(242, 92), Some(PlaylistButton::ListOpts));
        // The SEL 75-83 gap is a settled miss at y = 92 (S01 §4.2 round 3b).
        for x in 75..84 {
            assert_eq!(hit_test_button(x, 92), None, "x = {x}");
        }
        // Outside the button band nothing hits.
        assert_eq!(hit_test_button(24, 91), None);
        assert_eq!(hit_test_button(24, 110), None);
    }

    #[test]
    fn the_playlist_title_bar_has_no_window_menu_button() {
        // S06 §1: no `#32768` popup after the (6,3) click, so no button.
        assert!(!has_window_menu_button());
    }

    #[test]
    fn syncing_entries_keeps_live_selection_and_clamps_the_view() {
        let mut window = window_with(60);
        window.scroll_to(56);
        window.press_sel();
        // Entries 57-59 vanish; 56 stays selected, the view clamps.
        window.set_entries(
            ids(60)
                .into_iter()
                .filter(|id| *id != EntryId(57) && *id != EntryId(58) && *id != EntryId(59))
                .collect(),
        );
        assert_eq!(window.selected_ids(), vec![EntryId(56)]);
        assert_eq!(window.len(), 57);
        // The bottom page of 57 entries starts at offset 53.
        assert_eq!(window.first_visible(), 53);
    }

    #[test]
    fn double_click_row_selects_and_emits_play_entry() {
        let mut window = window_with(10);
        let cmd = window.double_click_row(2);
        assert_eq!(cmd, Some(Command::PlayEntry(EntryId(2))));
        assert_eq!(window.selected_ids(), vec![EntryId(2)]);

        // Out of bounds row returns None
        assert_eq!(window.double_click_row(VISIBLE_ROWS), None);
    }

    #[test]
    fn double_click_index_selects_and_emits_play_entry() {
        let mut window = window_with(10);
        let cmd = window.double_click_index(7);
        assert_eq!(cmd, Some(Command::PlayEntry(EntryId(7))));
        assert_eq!(window.selected_ids(), vec![EntryId(7)]);

        // Out of bounds index returns None
        assert_eq!(window.double_click_index(100), None);
    }

    #[test]
    fn apply_select_handles_all_select_ops() {
        let mut window = window_with(4);
        window.apply_select(&SelectOp::All);
        assert_eq!(
            window.selected_ids(),
            vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)]
        );

        window.apply_select(&SelectOp::None);
        assert!(window.selected_ids().is_empty());

        window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(3)]));
        assert_eq!(window.selected_ids(), vec![EntryId(1), EntryId(3)]);

        window.apply_select(&SelectOp::Invert);
        assert_eq!(window.selected_ids(), vec![EntryId(0), EntryId(2)]);

        window.apply_select(&SelectOp::Toggle(EntryId(0)));
        assert_eq!(window.selected_ids(), vec![EntryId(2)]);
    }

    /// Opening either dialog needs an OS window, so this never runs in the
    /// headless suite: it pins the two spec'd titles the real dialogs must
    /// carry (S06 §4) for the run that does open them.
    #[ignore = "opens an OS dialog window; run manually only"]
    #[test]
    fn opening_either_dialog_needs_an_os_window() {
        assert_eq!(PlaylistDialog::AddFiles.title(), ADD_DIALOG_TITLE);
        assert_eq!(PlaylistDialog::LoadPlaylist.title(), LOAD_DIALOG_TITLE);
    }

    #[test]
    fn up_and_down_move_the_selection_and_reveal_it() {
        // S03 §2 Up (`40289`) / Down (`40290`): provisional rows, implemented
        // as own choices (single selection, view follows).
        let mut window = window_with(60);
        window.scroll_to(56);
        window.apply_select(&SelectOp::Set(vec![EntryId(57)]));
        window.move_selection_up();
        assert_eq!(window.selected_ids(), vec![EntryId(56)]);
        assert_eq!(window.first_visible(), 56);
        // At the top of the view moving up scrolls the view along.
        window.move_selection_up();
        assert_eq!(window.selected_ids(), vec![EntryId(55)]);
        assert_eq!(window.first_visible(), 55);
        window.move_selection_down();
        assert_eq!(window.selected_ids(), vec![EntryId(56)]);
        assert_eq!(window.first_visible(), 55);
        // Clamped at the ends of the list.
        let mut window = window_with(3);
        window.apply_select(&SelectOp::Set(vec![EntryId(0)]));
        window.move_selection_up();
        assert_eq!(window.selected_ids(), vec![EntryId(0)]);
        window.apply_select(&SelectOp::Set(vec![EntryId(2)]));
        window.move_selection_down();
        assert_eq!(window.selected_ids(), vec![EntryId(2)]);
        // An empty list stays empty.
        let mut window = PlaylistWindow::new();
        window.move_selection_up();
        window.move_selection_down();
        assert!(window.selected_ids().is_empty());
    }

    #[test]
    fn home_and_end_jump_to_the_ends_of_the_list() {
        // S03 §2 Home (`40264`) / End (`40265`): provisional rows, implemented
        // as own choices (single selection, view follows).
        let mut window = window_with(60);
        window.scroll_to(56);
        window.jump_to_first();
        assert_eq!(window.selected_ids(), vec![EntryId(0)]);
        assert_eq!(window.first_visible(), 0);
        window.jump_to_last();
        assert_eq!(window.selected_ids(), vec![EntryId(59)]);
        assert_eq!(window.first_visible(), 56);
        let mut window = PlaylistWindow::new();
        window.jump_to_first();
        window.jump_to_last();
        assert!(window.selected_ids().is_empty());
    }

    #[test]
    fn enter_plays_the_first_selected_entry() {
        // S03 §2 Enter (`40184`) is menu-confirmed; which entry plays with
        // several selected is not specified, so the first selected in
        // playlist order plays (own choice).
        let mut window = window_with(6);
        assert_eq!(window.play_selected(), None);
        window.apply_select(&SelectOp::Set(vec![EntryId(3), EntryId(1)]));
        assert_eq!(window.play_selected(), Some(Command::PlayEntry(EntryId(1))));
        assert_eq!(window.selected_ids(), vec![EntryId(1)]);
    }

    #[test]
    fn the_playlist_key_map_covers_the_provisional_rows_and_enter() {
        use crate::shortcuts::{Key, Modifiers};

        // The implemented S03 §2 rows, each with no modifiers.
        assert_eq!(
            playlist_key_action(Key::Up, Modifiers::NONE),
            Some(PlaylistKeyAction::MoveSelectionUp)
        );
        assert_eq!(
            playlist_key_action(Key::Down, Modifiers::NONE),
            Some(PlaylistKeyAction::MoveSelectionDown)
        );
        assert_eq!(
            playlist_key_action(Key::Home, Modifiers::NONE),
            Some(PlaylistKeyAction::JumpToFirst)
        );
        assert_eq!(
            playlist_key_action(Key::End, Modifiers::NONE),
            Some(PlaylistKeyAction::JumpToLast)
        );
        assert_eq!(
            playlist_key_action(Key::PageUp, Modifiers::NONE),
            Some(PlaylistKeyAction::PageUp)
        );
        assert_eq!(
            playlist_key_action(Key::PageDown, Modifiers::NONE),
            Some(PlaylistKeyAction::PageDown)
        );
        assert_eq!(
            playlist_key_action(Key::Enter, Modifiers::NONE),
            Some(PlaylistKeyAction::PlaySelected)
        );
        // Modifiers take the shared path: a chord S03 §2 does not bind stays
        // unmapped here.
        assert_eq!(playlist_key_action(Key::Up, Modifiers::CTRL), None);
        assert_eq!(playlist_key_action(Key::Enter, Modifiers::CTRL), None);
        assert_eq!(
            playlist_key_action(Key::Letter(b'A'), Modifiers::CTRL.union(Modifiers::ALT)),
            Some(PlaylistKeyAction::Shared(
                crate::shortcuts::ShortcutAction::ToggleAlwaysOnTop
            ))
        );
        assert_eq!(playlist_key_action(Key::Delete, Modifiers::SHIFT), None);

        // The four reorder rows of S03 §2, menu-confirmed: `Ctrl+Shift+1/2/3`
        // for the three sorts and `Ctrl+Shift+R` for randomizing.
        let chord = Modifiers::CTRL.union(Modifiers::SHIFT);
        assert_eq!(
            playlist_key_action(Key::Digit(1), chord),
            Some(PlaylistKeyAction::SortByTitle)
        );
        assert_eq!(
            playlist_key_action(Key::Digit(2), chord),
            Some(PlaylistKeyAction::SortByFileName)
        );
        assert_eq!(
            playlist_key_action(Key::Digit(3), chord),
            Some(PlaylistKeyAction::SortByPathAndFileName)
        );
        assert_eq!(
            playlist_key_action(Key::Letter(b'R'), chord),
            Some(PlaylistKeyAction::Randomize)
        );
        // Only the full chord matches: neither modifier alone, nor the chord
        // on a key the table does not bind.
        assert_eq!(playlist_key_action(Key::Digit(1), Modifiers::CTRL), None);
        assert_eq!(playlist_key_action(Key::Digit(1), Modifiers::SHIFT), None);
        assert_eq!(playlist_key_action(Key::Digit(1), Modifiers::NONE), None);
        assert_eq!(playlist_key_action(Key::Digit(4), chord), None);
        assert_eq!(playlist_key_action(Key::Letter(b'A'), chord), None);
    }

    #[test]
    fn the_playlist_key_map_covers_remove_dead_and_clear() {
        use crate::shortcuts::{Key, Modifiers};

        // S03 §2: Alt+Delete is `40222` ("Remove all dead files", RFC 0018).
        assert_eq!(
            playlist_key_action(Key::Delete, Modifiers::ALT),
            Some(PlaylistKeyAction::RemoveDead)
        );
        // S03 §2 binds the one id `40214` to both Ctrl+Shift+Delete and Ctrl+N.
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            playlist_key_action(Key::Delete, ctrl_shift),
            Some(PlaylistKeyAction::ClearPlaylist)
        );
        assert_eq!(
            playlist_key_action(Key::Letter(b'N'), Modifiers::CTRL),
            Some(PlaylistKeyAction::ClearPlaylist)
        );
        // Neighbouring chords stay unmapped.
        assert_eq!(playlist_key_action(Key::Delete, Modifiers::SHIFT), None);
        assert_eq!(
            playlist_key_action(Key::Delete, Modifiers::CTRL | Modifiers::ALT),
            None
        );
        assert_eq!(playlist_key_action(Key::Letter(b'N'), ctrl_shift), None);
        // Ctrl+Delete keeps cropping; plain Delete removes the selection.
        assert_eq!(
            playlist_key_action(Key::Delete, Modifiers::CTRL),
            Some(PlaylistKeyAction::Crop)
        );
        assert_eq!(
            playlist_key_action(Key::Delete, Modifiers::NONE),
            Some(PlaylistKeyAction::Delete)
        );
    }

    /// The S03 §2 rows, written out independently of the table:
    /// (key, modifiers, command id).
    fn s03_playlist_rows() -> Vec<(Key, Modifiers, u32)> {
        let n = Modifiers::NONE;
        let c = Modifiers::CTRL;
        let a = Modifiers::ALT;
        let s = Modifiers::SHIFT;
        let cs = c | s;
        let ca = c | a;
        vec![
            (Key::Digit(1), cs, 40209),
            (Key::Digit(2), cs, 40210),
            (Key::Digit(3), cs, 40211),
            (Key::Digit(3), a, 40208),
            (Key::Letter(b'A'), c, 40205),
            (Key::Letter(b'A'), ca, 40019),
            (Key::Letter(b'B'), n, 40048),
            (Key::Letter(b'C'), n, 40046),
            (Key::Letter(b'D'), c, 40165),
            (Key::Letter(b'E'), c, 40255),
            (Key::Letter(b'E'), a, 40040),
            (Key::Letter(b'E'), ca, 40293),
            (Key::Letter(b'F'), a, 40043),
            (Key::Letter(b'G'), a, 40036),
            (Key::Letter(b'G'), ca, 40292),
            (Key::Letter(b'I'), c, 40171),
            (Key::Letter(b'I'), a, 40319),
            (Key::Letter(b'J'), n, 40194),
            (Key::Letter(b'J'), c, 40193),
            (Key::Letter(b'K'), c, 40191),
            (Key::Letter(b'K'), a, 40221),
            (Key::Letter(b'K'), cs, 40192),
            (Key::Letter(b'L'), n, 1032),
            (Key::Letter(b'L'), c, 1039),
            (Key::Letter(b'L'), s, 1036),
            (Key::Letter(b'M'), a, 40334),
            (Key::Letter(b'N'), c, 40214),
            (Key::Letter(b'N'), ca, 40305),
            (Key::Letter(b'O'), c, 40202),
            (Key::Letter(b'O'), a, 40317),
            (Key::Letter(b'P'), c, 40012),
            (Key::Letter(b'R'), n, 40022),
            (Key::Letter(b'R'), c, 40213),
            (Key::Letter(b'R'), cs, 40212),
            (Key::Letter(b'S'), n, 40023),
            (Key::Letter(b'S'), c, 40204),
            (Key::Letter(b'S'), a, 40219),
            (Key::Letter(b'T'), c, 40242),
            (Key::Letter(b'T'), a, 40298),
            (Key::Letter(b'V'), n, 40047),
            (Key::Letter(b'V'), c, 40157),
            (Key::Letter(b'V'), a, 40328),
            (Key::Letter(b'V'), s, 40147),
            (Key::NumAdd, c, 40335),
            (Key::NumSubtract, c, 40336),
            (Key::NumDecimal, n, 40194),
            (Key::Delete, n, 1034),
            (Key::Delete, c, 1035),
            (Key::Delete, a, 40222),
            (Key::Delete, cs, 40214),
            (Key::Down, n, 40290),
            (Key::Down, a, 40216),
            (Key::Up, n, 40289),
            (Key::Up, a, 40215),
            (Key::Home, n, 40264),
            (Key::End, n, 40265),
            (Key::PageUp, n, 40256),
            (Key::PageDown, n, 40257),
            (Key::Enter, n, 40184),
            (Key::Insert, n, 1036),
            (Key::Insert, c, 1032),
            (Key::Insert, s, 1039),
            (Key::F(1), n, 40041),
            (Key::F(3), n, 40318),
            (Key::F(4), c, 40224),
            (Key::F(5), n, 40291),
            (Key::Left, n, 40061),
            (Key::Right, n, 40060),
            (Key::Numpad(0), n, 1032),
            (Key::Numpad(0), c, 1039),
            (Key::Numpad(1), n, 40197),
            (Key::Numpad(2), n, 40059),
            (Key::Numpad(3), n, 40195),
            (Key::Numpad(4), n, 40044),
            (Key::Numpad(5), n, 40045),
            (Key::Numpad(6), n, 40048),
            (Key::Numpad(7), n, 40061),
            (Key::Numpad(8), n, 40058),
            (Key::Numpad(9), n, 40060),
            (Key::Enter, a, 40337),
            (Key::Tab, c, 40063),
            (Key::Letter(b'W'), c, 40064),
            (Key::Letter(b'W'), a, 40258),
            (Key::Letter(b'X'), n, 40045),
            (Key::Letter(b'X'), c, 40185),
            (Key::Letter(b'X'), s, 40029),
            (Key::Letter(b'Z'), n, 40044),
            (Key::Letter(b'Z'), c, 40154),
        ]
    }

    #[test]
    fn every_s03_playlist_row_maps_to_an_action_or_is_listed_as_unimplemented() {
        let rows = s03_playlist_rows();
        assert_eq!(rows.len(), 88, "S03 §2 has 88 rows");
        let mut seen = std::collections::HashSet::new();
        for (key, modifiers, id) in rows {
            assert!(
                seen.insert((key, modifiers)),
                "S03 §2 binds {key:?} with {modifiers:?} twice"
            );
            let binding = playlist_key_binding(key, modifiers);
            let unimplemented = unimplemented_playlist_row(key, modifiers);
            assert!(
                binding.is_some() || unimplemented.is_some(),
                "S03 §2 binds {key:?} with {modifiers:?} (id {id}), but the table does not"
            );
            assert!(
                binding.is_none() || unimplemented.is_none(),
                "{key:?} with {modifiers:?} is both bound and listed as unimplemented"
            );
            if let Some(binding) = binding {
                assert_eq!(binding.action.command_id(), id, "{key:?} {modifiers:?}");
            }
            if let Some(row) = unimplemented {
                assert_eq!(row.command_id, id, "{key:?} {modifiers:?}");
            }
        }
        assert_eq!(
            PLAYLIST_TABLE.len() + UNIMPLEMENTED_PLAYLIST_ROWS.len(),
            88,
            "the table and the unimplemented rows cover S03 §2 exactly"
        );
    }

    #[test]
    fn the_playlist_table_has_no_duplicate_chords() {
        for (index, first) in PLAYLIST_TABLE.iter().enumerate() {
            for second in &PLAYLIST_TABLE[index + 1..] {
                assert!(
                    !(first.key == second.key && first.modifiers == second.modifiers),
                    "{:?} is bound twice",
                    first.key
                );
            }
        }
        for (index, first) in UNIMPLEMENTED_PLAYLIST_ROWS.iter().enumerate() {
            for second in &UNIMPLEMENTED_PLAYLIST_ROWS[index + 1..] {
                assert!(
                    !(first.key == second.key && first.modifiers == second.modifiers),
                    "{:?} is listed twice",
                    first.key
                );
            }
        }
        // A chord that is bound is never listed as unimplemented as well.
        for row in UNIMPLEMENTED_PLAYLIST_ROWS {
            assert!(
                playlist_key_binding(row.key, row.modifiers).is_none(),
                "{:?} is both bound and listed as unimplemented",
                row.key
            );
        }
    }

    #[test]
    fn the_add_rows_name_the_matching_media_face() {
        // S03 §2: `1032` "Add file(s)", `1036` "Add directory" and `1039`
        // "Add location" — the playlist editor's own add commands.
        for (key, modifiers, expected) in [
            (
                Key::Letter(b'L'),
                Modifiers::NONE,
                PlaylistKeyAction::AddFiles,
            ),
            (
                Key::Letter(b'L'),
                Modifiers::CTRL,
                PlaylistKeyAction::AddLocation,
            ),
            (
                Key::Letter(b'L'),
                Modifiers::SHIFT,
                PlaylistKeyAction::AddDirectory,
            ),
            (
                Key::Insert,
                Modifiers::NONE,
                PlaylistKeyAction::AddDirectory,
            ),
            (Key::Insert, Modifiers::CTRL, PlaylistKeyAction::AddFiles),
            (
                Key::Insert,
                Modifiers::SHIFT,
                PlaylistKeyAction::AddLocation,
            ),
            (Key::Numpad(0), Modifiers::NONE, PlaylistKeyAction::AddFiles),
            (
                Key::Numpad(0),
                Modifiers::CTRL,
                PlaylistKeyAction::AddLocation,
            ),
        ] {
            assert_eq!(
                playlist_key_action(key, modifiers),
                Some(expected),
                "{key:?} {modifiers:?}"
            );
        }
        assert_eq!(PlaylistKeyAction::AddFiles.command_id(), 1032);
        assert_eq!(PlaylistKeyAction::AddDirectory.command_id(), 1036);
        assert_eq!(PlaylistKeyAction::AddLocation.command_id(), 1039);
    }

    #[test]
    fn the_playlist_rows_resolve_to_their_own_actions() {
        for (key, modifiers, expected) in [
            (
                Key::Letter(b'A'),
                Modifiers::CTRL,
                PlaylistKeyAction::SelectAll,
            ),
            (
                Key::Letter(b'I'),
                Modifiers::CTRL,
                PlaylistKeyAction::InvertSelection,
            ),
            (
                Key::Letter(b'S'),
                Modifiers::CTRL,
                PlaylistKeyAction::SavePlaylist,
            ),
            (
                Key::Letter(b'O'),
                Modifiers::CTRL,
                PlaylistKeyAction::OpenPlaylist,
            ),
            (
                Key::Letter(b'R'),
                Modifiers::CTRL,
                PlaylistKeyAction::Reverse,
            ),
            (Key::Digit(3), Modifiers::ALT, PlaylistKeyAction::FileInfo),
            (
                Key::Letter(b'I'),
                Modifiers::ALT,
                PlaylistKeyAction::BookmarkSelection,
            ),
            (Key::F(4), Modifiers::CTRL, PlaylistKeyAction::CloseEditor),
            (
                Key::Letter(b'W'),
                Modifiers::CTRL,
                PlaylistKeyAction::ToggleEditorShade,
            ),
            (Key::Delete, Modifiers::NONE, PlaylistKeyAction::Delete),
            (Key::Delete, Modifiers::CTRL, PlaylistKeyAction::Crop),
            (Key::Delete, Modifiers::ALT, PlaylistKeyAction::RemoveDead),
            (
                Key::Delete,
                Modifiers::CTRL.union(Modifiers::SHIFT),
                PlaylistKeyAction::ClearPlaylist,
            ),
            (
                Key::Letter(b'N'),
                Modifiers::CTRL,
                PlaylistKeyAction::ClearPlaylist,
            ),
            (Key::Up, Modifiers::ALT, PlaylistKeyAction::MoveBlockUp),
            (Key::Down, Modifiers::ALT, PlaylistKeyAction::MoveBlockDown),
        ] {
            assert_eq!(
                playlist_key_action(key, modifiers),
                Some(expected),
                "{key:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn the_shared_rows_resolve_to_the_shared_actions() {
        use crate::shortcuts::ShortcutAction as A;
        for (key, modifiers, expected) in [
            (Key::Letter(b'B'), Modifiers::NONE, A::NextTrack),
            (Key::Letter(b'C'), Modifiers::NONE, A::PauseUnpause),
            (Key::Letter(b'V'), Modifiers::NONE, A::Stop),
            (Key::Letter(b'X'), Modifiers::NONE, A::Play),
            (Key::Letter(b'Z'), Modifiers::NONE, A::PreviousTrack),
            (Key::Letter(b'R'), Modifiers::NONE, A::ToggleRepeat),
            (Key::Letter(b'S'), Modifiers::NONE, A::ToggleShuffle),
            (Key::Left, Modifiers::NONE, A::Rewind5s),
            (Key::Right, Modifiers::NONE, A::Forward5s),
            (Key::F(1), Modifiers::NONE, A::About),
            (Key::F(3), Modifiers::NONE, A::PlaylistSearch),
            (Key::F(5), Modifiers::NONE, A::RefreshSkin),
            (Key::Tab, Modifiers::CTRL, A::NextWindow),
            (Key::Enter, Modifiers::ALT, A::ToggleVideoFullscreen),
            (Key::Letter(b'W'), Modifiers::ALT, A::ShowMainWindow),
            (Key::Numpad(1), Modifiers::NONE, A::TenTracksBack),
            (Key::Numpad(2), Modifiers::NONE, A::VolumeDown),
            (Key::Numpad(3), Modifiers::NONE, A::TenTracksForward),
            (Key::Numpad(4), Modifiers::NONE, A::PreviousTrack),
            (Key::Numpad(5), Modifiers::NONE, A::Play),
            (Key::Numpad(6), Modifiers::NONE, A::NextTrack),
            (Key::Numpad(7), Modifiers::NONE, A::Rewind5s),
            (Key::Numpad(8), Modifiers::NONE, A::VolumeUp),
            (Key::Numpad(9), Modifiers::NONE, A::Forward5s),
            (Key::NumDecimal, Modifiers::NONE, A::JumpToFile),
            (Key::Letter(b'J'), Modifiers::NONE, A::JumpToFile),
            (Key::Letter(b'J'), Modifiers::CTRL, A::JumpToTime),
            (Key::Letter(b'X'), Modifiers::CTRL, A::OpenLocation),
            (Key::Letter(b'X'), Modifiers::SHIFT, A::OpenFiles),
            (Key::Letter(b'Z'), Modifiers::CTRL, A::StartOfList),
            (
                Key::Letter(b'A'),
                Modifiers::CTRL.union(Modifiers::ALT),
                A::ToggleAlwaysOnTop,
            ),
            (Key::Letter(b'D'), Modifiers::CTRL, A::ToggleDoubleSize),
            (Key::Letter(b'E'), Modifiers::ALT, A::TogglePlaylistEditor),
            (Key::Letter(b'F'), Modifiers::ALT, A::OpenMainMenu),
            (Key::Letter(b'G'), Modifiers::ALT, A::ToggleEqualizer),
            (Key::Letter(b'K'), Modifiers::CTRL, A::SelectVisPlugin),
            (Key::Letter(b'K'), Modifiers::ALT, A::ConfigureVisPlugin),
            (
                Key::Letter(b'K'),
                Modifiers::CTRL.union(Modifiers::SHIFT),
                A::ToggleVisPlugin,
            ),
            (Key::Letter(b'M'), Modifiers::ALT, A::Minimize),
            (
                Key::Letter(b'N'),
                Modifiers::CTRL.union(Modifiers::ALT),
                A::NewInstance,
            ),
            (Key::Letter(b'O'), Modifiers::ALT, A::VisOptions),
            (Key::Letter(b'P'), Modifiers::CTRL, A::Preferences),
            (Key::Letter(b'S'), Modifiers::ALT, A::SkinBrowser),
            (Key::Letter(b'T'), Modifiers::CTRL, A::ToggleTimeRemaining),
            (Key::Letter(b'T'), Modifiers::ALT, A::ToggleMinibrowser),
            (Key::Letter(b'V'), Modifiers::CTRL, A::StopAfterCurrent),
            (Key::Letter(b'V'), Modifiers::ALT, A::ToggleVideoWindow),
            (Key::Letter(b'V'), Modifiers::SHIFT, A::StopWithFadeout),
        ] {
            assert_eq!(
                playlist_key_action(key, modifiers),
                Some(PlaylistKeyAction::Shared(expected)),
                "{key:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn the_playlist_table_shadows_the_main_and_global_rows() {
        use crate::shortcuts::{ShortcutWindow, lookup};

        // Ctrl+A: the playlist selects all (`40205`), the main and global
        // tables toggle "always on top" (`40019`).
        assert_eq!(
            playlist_key_action(Key::Letter(b'A'), Modifiers::CTRL),
            Some(PlaylistKeyAction::SelectAll)
        );
        assert_eq!(
            lookup(ShortcutWindow::Main, Key::Letter(b'A'), Modifiers::CTRL)
                .map(ShortcutAction::command_id),
            Some(40019)
        );
        // Ctrl+L: the playlist adds a location (`1039`), the main table opens
        // one (`40185`) and the global table a minibrowser command (`40302`).
        assert_eq!(
            playlist_key_action(Key::Letter(b'L'), Modifiers::CTRL),
            Some(PlaylistKeyAction::AddLocation)
        );
        // Ctrl+O: the playlist opens a playlist (`40202`), the global table a
        // minibrowser command (`40303`).
        assert_eq!(
            playlist_key_action(Key::Letter(b'O'), Modifiers::CTRL),
            Some(PlaylistKeyAction::OpenPlaylist)
        );
        // Ctrl+R: the playlist reverses the list (`40213`), the global table a
        // minibrowser command (`40301`).
        assert_eq!(
            playlist_key_action(Key::Letter(b'R'), Modifiers::CTRL),
            Some(PlaylistKeyAction::Reverse)
        );
        // Up: the playlist moves the selection (`40289`), the main table the
        // volume (`40058`).
        assert_eq!(
            playlist_key_action(Key::Up, Modifiers::NONE),
            Some(PlaylistKeyAction::MoveSelectionUp)
        );
        // Ctrl+E has no command yet, so it is listed as unimplemented and
        // never reaches the main table's `40186` "Toggle easy move".
        assert_eq!(
            playlist_key_action(Key::Letter(b'E'), Modifiers::CTRL),
            None
        );
        assert_eq!(
            unimplemented_playlist_row(Key::Letter(b'E'), Modifiers::CTRL)
                .map(|row| row.command_id),
            Some(40255)
        );
    }

    #[test]
    fn the_provisional_playlist_rows_are_flagged() {
        let provisional: Vec<u32> = PLAYLIST_TABLE
            .iter()
            .filter(|binding| binding.provisional)
            .map(|binding| binding.action.command_id())
            .collect();
        // Alt+F, Alt+M, Ctrl+Alt+N, Num2, Num8, Down, Alt+Down, Up, Alt+Up,
        // Home, End, PgUp, PgDn, F3, Ctrl+F4, F5, Alt+Enter, Ctrl+Tab.
        assert_eq!(provisional.len(), 18);
        for id in [
            40043, 40334, 40305, 40059, 40058, 40290, 40216, 40289, 40215, 40264, 40265, 40256,
            40257, 40318, 40224, 40291, 40337, 40063,
        ] {
            assert!(provisional.contains(&id), "{id}");
        }
        // The two font-size rows are provisional too, and unimplemented.
        for id in [40335, 40336] {
            assert!(
                UNIMPLEMENTED_PLAYLIST_ROWS
                    .iter()
                    .any(|row| row.command_id == id && row.provisional),
                "{id}"
            );
        }
    }

    #[test]
    fn the_unimplemented_rows_are_the_five_s03_rows_without_a_command() {
        let rows: Vec<u32> = UNIMPLEMENTED_PLAYLIST_ROWS
            .iter()
            .map(|row| row.command_id)
            .collect();
        assert_eq!(rows, vec![40255, 40293, 40292, 40335, 40336]);
        // None of them resolves to an action.
        for row in UNIMPLEMENTED_PLAYLIST_ROWS {
            assert_eq!(
                playlist_key_action(row.key, row.modifiers),
                None,
                "{:?}",
                row.key
            );
        }
    }

    #[test]
    fn alt_up_and_alt_down_move_the_selected_block() {
        // S03 §2: `40215`/`40216` "Move the selected entry up/down"
        // (provisional). The model emits the ordinary `MoveEntries` command.
        let mut window = window_with(4);
        assert_eq!(window.move_block_up(), None, "nothing selected");
        assert_eq!(window.move_block_down(), None, "nothing selected");
        window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(2)]));
        assert_eq!(
            window.move_block_up(),
            Some(Command::MoveEntries {
                ids: vec![EntryId(1), EntryId(2)],
                to: 0,
            })
        );
        assert_eq!(
            window.move_block_down(),
            Some(Command::MoveEntries {
                ids: vec![EntryId(1), EntryId(2)],
                to: 2,
            })
        );
        // The first row cannot move up and the last row cannot move down.
        let mut window = window_with(3);
        window.apply_select(&SelectOp::Set(vec![EntryId(0)]));
        assert_eq!(window.move_block_up(), None);
        window.apply_select(&SelectOp::Set(vec![EntryId(2)]));
        assert_eq!(window.move_block_down(), None);
    }
}
