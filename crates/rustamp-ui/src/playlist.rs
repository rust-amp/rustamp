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
//!
//! The **selection gestures** are own choices as well. S06 §2 settles only the
//! single click (one row) and SEL (all visible rows), and S06 §8 Q11/Q14
//! record that "beyond the all-select of §2 nothing about SEL visuals or
//! multi-selection is settled". The shift-range, the ctrl-toggle and the
//! keyboard extension below are therefore *this implementation's* choice, not
//! measurements. They mirror the reference semantics of `rustamp-playlist`'s
//! `interact` module — a contiguous range from an anchor, a one-row toggle, an
//! anchor clamped into the list, an empty list refusing the gesture — which
//! `rustamp-ui` may not depend on (S06i3), so the rules are restated here and
//! cited there.

use std::collections::BTreeSet;

use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
use rustamp_core::playlist::EntryId;

use crate::playlist_font::PlaylistFontSize;
use crate::shortcuts::{Key, Modifiers, ShortcutAction};

/// How many list rows the settled page holds (S06 §3).
///
/// The 1501/1501 px arrow symmetry is settled; the "four rows" count itself is
/// a **visual reading** (OCR of the uncertain skin font), so the constant is
/// marked accordingly.
///
/// The count is the one of the **default** font size: the row height follows
/// the font, so the page holds more rows with a smaller font and fewer with a
/// bigger one (see [`crate::playlist_window::visible_rows`]). The page step
/// is therefore the page's row count, not a constant.
pub const VISIBLE_ROWS: usize = 4;

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
/// reorder rows (the three sorts and randomize). The two provisional
/// font-size rows (`40335` / `40336`) change the rows the editor draws and
/// nothing else, so they are UI-local (RFC 0005 §9/§11.6).
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
    /// Ctrl+Alt+G: generate an HTML playlist (`40292`, menu-confirmed).
    ///
    /// The document is written where the user says: the app records the
    /// request and the presenter asks for a file, so the row carries no core
    /// command (writing a file is not a player action, RFC 0005 §9).
    GenerateHtmlPlaylist,
    /// Ctrl+Alt+E: read the extended info of the selection (`40293`,
    /// menu-confirmed).
    ///
    /// The action reads the tags of the selected entries and shows them in
    /// the extended-info dialog. S03 §2 gives the id and the chord only, so
    /// the content is an **own choice** (see
    /// [`crate::dialogs::ExtendedInfo`]). No core command opens that dialog
    /// (reading a file's tags is not a player action, RFC 0005 §9), so the
    /// app opens it itself.
    ReadExtendedInfo,
    /// Ctrl+E: edit the playlist entry (`40255`, menu-confirmed).
    ///
    /// The action edits the **tags of the selected entry**: the dialog model
    /// is [`crate::dialogs::EditEntryDialog`], which shows a field per tag
    /// and reports the changed fields. S03 §2 gives the id and the chord
    /// only, so the dialog's content is an **own choice**. No core command
    /// opens that dialog (writing a file's tags is not a player action,
    /// RFC 0005 §9), so the app opens it itself.
    EditEntry,
    /// Ctrl+F4: close the playlist editor (`40224`, provisional).
    CloseEditor,
    /// Ctrl+W: toggle the editor's windowshade (`40064`).
    ///
    /// S03 §2 binds the global "toggle windowshade" id, but the editor's
    /// shade is its own (S01 §1: a shaded editor is one row tall, and the
    /// editor's title bar carries its own shade button), so the row toggles
    /// the editor and not the main window.
    ToggleEditorShade,
    /// Ctrl+Num+: increase the playlist font size (`40335`, provisional).
    ///
    /// A view setting of this window: it changes the rows the editor draws
    /// and nothing else, so it needs no core command (RFC 0005 §9/§11.6).
    IncreaseFontSize,
    /// Ctrl+Num-: decrease the playlist font size (`40336`, provisional).
    ///
    /// The counterpart of [`Self::IncreaseFontSize`].
    DecreaseFontSize,
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
            Self::GenerateHtmlPlaylist => 40292,
            Self::ReadExtendedInfo => 40293,
            Self::EditEntry => 40255,
            Self::CloseEditor => 40224,
            Self::ToggleEditorShade => 40064,
            Self::IncreaseFontSize => 40335,
            Self::DecreaseFontSize => 40336,
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
/// The 88 rows whose command this build can act on — every row of S03 §2.
/// The rows this build cannot act on would be listed in
/// [`UNIMPLEMENTED_PLAYLIST_ROWS`], which is empty.
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
        Modifiers::CTRL,
        PlaylistKeyAction::EditEntry,
        false,
    ),
    prow(
        Key::Letter(b'E'),
        Modifiers::ALT,
        PlaylistKeyAction::Shared(ShortcutAction::TogglePlaylistEditor),
        false,
    ),
    prow(
        Key::Letter(b'E'),
        Modifiers::CTRL.union(Modifiers::ALT),
        PlaylistKeyAction::ReadExtendedInfo,
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
        Key::Letter(b'G'),
        Modifiers::CTRL.union(Modifiers::ALT),
        PlaylistKeyAction::GenerateHtmlPlaylist,
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
        Key::NumAdd,
        Modifiers::CTRL,
        PlaylistKeyAction::IncreaseFontSize,
        true,
    ),
    prow(
        Key::NumSubtract,
        Modifiers::CTRL,
        PlaylistKeyAction::DecreaseFontSize,
        true,
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
/// The list is **empty**: every row of S03 §2 has an action now. The last
/// gap was `Ctrl+E` (`40255` "Edit the playlist entry"), which the
/// [`PlaylistKeyAction::EditEntry`] row fills: the dialog model is
/// [`crate::dialogs::EditEntryDialog`].
///
/// The `Ctrl+Alt+E` row (`40293` "Read extended info on selection") is a
/// bound row too: it reads the tags of the selected entries and shows them
/// (see [`PlaylistKeyAction::ReadExtendedInfo`]).
///
/// The two font-size rows (`40335` / `40336`) are bound rows as well: they
/// are view settings of the editor and act on the playlist machine alone
/// (RFC 0005 §9/§11.6), so they are rows of [`PLAYLIST_TABLE`].
pub const UNIMPLEMENTED_PLAYLIST_ROWS: &[UnimplementedPlaylistRow] = &[];

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
/// slice is as many entries as the list page holds at the current font size
/// ([`PlaylistWindow::visible_rows`]), starting at the scroll offset.
#[derive(Debug, Clone, Default)]
pub struct PlaylistWindow {
    /// The info field: the length of the selection and of the list.
    pub info_text: String,
    /// The time field's four digits while a track is open.
    pub time_digits: Option<[u8; 4]>,
    /// Render the skin's 275x14 playlist windowshade band (S01 §1/§4).
    pub windowshade: bool,
    /// Held title button, overlaid from pledit.bmp; bottom buttons have no
    /// pressed sprite (S01 §4.1).
    pub pressed_title_button: Option<crate::playlist_window::PlaylistTitleButton>,
    /// The runtime font size (S03 §2 `40335` / `40336`).
    font_size: PlaylistFontSize,
    entries: Vec<EntryId>,
    first_visible: usize,
    selected: BTreeSet<EntryId>,
    /// The fixed end of the range gestures: the entry of the last click that
    /// set one (own choice; S06 §2 settles only the single click).
    shift_anchor: Option<EntryId>,
    /// The moving end of the range gestures: the entry the user pointed at
    /// last. The anchor and the focus together span the selected range.
    shift_focus: Option<EntryId>,
    current: Option<EntryId>,
    drag: Option<PlaylistDrag>,
    /// The scrollbar handle drag: how far below the handle's top the pointer
    /// took hold of it, while the handle is held.
    ///
    /// `None` when no handle is held. The grab offset keeps the handle under
    /// the pointer exactly as it was grabbed, so taking hold of the handle
    /// never moves the view (own choice: S06 §3 settles that a dragged
    /// handle moves the view, not how the grab behaves).
    scrollbar_grab: Option<i32>,
    /// Client dimensions; `None` retains the classic default size.
    size: Option<(u32, u32)>,
}

impl PlaylistWindow {
    /// An empty playlist window showing nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Syncs the entry order (the app is the state owner).
    ///
    /// Entries that no longer exist drop out of the selection; the scroll
    /// offset is clamped so the visible slice stays inside the list. The
    /// anchor of the range gestures is dropped with its entry, so a stale
    /// anchor can never name a row that is not there.
    pub fn set_entries(&mut self, entries: Vec<EntryId>) {
        if self.entries != entries {
            self.cancel_drag();
        }
        self.entries = entries;
        let live: BTreeSet<EntryId> = self.entries.iter().copied().collect();
        self.selected.retain(|id| live.contains(id));
        if self
            .shift_anchor
            .is_some_and(|anchor| !live.contains(&anchor))
        {
            self.shift_anchor = None;
            self.shift_focus = None;
        }
        if self.shift_focus.is_some_and(|focus| !live.contains(&focus)) {
            self.shift_focus = None;
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

    /// The runtime font size (S03 §2 `40335` / `40336`).
    ///
    /// The size is the skin's (`pledit.txt`) until the user changes it; the
    /// user's choice replaces the size only, the family and the style stay
    /// the skin's.
    pub fn font_size(&self) -> PlaylistFontSize {
        self.font_size
    }

    /// The row height in pixels, from the font size.
    pub fn row_pitch(&self) -> i32 {
        crate::playlist_window::row_pitch(self.font_size.size())
    }

    /// How many rows the list page holds at the current font size.
    pub fn visible_rows(&self) -> usize {
        let height = self.size().1.saturating_sub(PLAYLIST_WINDOW_SIZE.1);
        ((crate::playlist_window::LIST_PAGE_HEIGHT + height as i32) / self.row_pitch()).max(1)
            as usize
    }

    /// Set the unshaded client size and clamp the scroll offset to the new page.
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = Some((width.clamp(275, 4096), height.clamp(116, 4096)));
        self.clamp_scroll();
    }

    /// The unshaded playlist client size.
    pub fn size(&self) -> (u32, u32) {
        self.size.unwrap_or(PLAYLIST_WINDOW_SIZE)
    }

    /// Whether a point is inside the current list area.
    pub fn contains_list_point(&self, x: i32, y: i32) -> bool {
        let (width, height) = self.size();
        (12..width as i32 - 21).contains(&x) && (23..height as i32 - 23).contains(&y)
    }

    /// The visible row under a point in the current list area.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.contains_list_point(x, y) {
            return None;
        }
        // The rows' pitch starts one pixel above the band (see `row_rect`).
        let row = ((y - 22) / self.row_pitch()) as usize;
        (row < self.visible_rows()).then_some(row)
    }

    /// Sets the skin's font size; the playlist uses it until the user
    /// changes the size.
    ///
    /// The app calls this when the skin changes, so the rows follow the
    /// skin's `Font=` line (S17 §5).
    pub fn set_skin_font_size(&mut self, size: u32) {
        self.font_size.set_skin_size(size);
    }

    /// Restores the user's chosen size, or clears it with `None`.
    ///
    /// This is the path a persisted choice comes back through.
    pub fn set_font_size_override(&mut self, size: Option<u32>) {
        self.font_size.set_user_size(size);
    }

    /// One step larger (S03 §2 `40335`, provisional).
    ///
    /// Returns whether the size changed; at the largest size it does not.
    pub fn increase_font_size(&mut self) -> bool {
        self.font_size.increase()
    }

    /// One step smaller (S03 §2 `40336`, provisional).
    ///
    /// Returns whether the size changed; at the smallest size it does not.
    pub fn decrease_font_size(&mut self) -> bool {
        self.font_size.decrease()
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

    /// The currently visible entries, at most the page's row count (S06 §3).
    pub fn visible_ids(&self) -> Vec<EntryId> {
        self.entries
            .iter()
            .skip(self.first_visible)
            .take(self.visible_rows())
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
    ///
    /// The **modifiers** choose the gesture (own choice; S06 §2 settles only
    /// the single click):
    ///
    /// * no modifier: the plain click of S06 §2 — the row alone replaces the
    ///   selection and becomes the anchor of the range gestures;
    /// * `Shift`: the contiguous range between the anchor and the pressed row
    ///   replaces the selection, and the anchor stays;
    /// * `Ctrl`: exactly the pressed row is toggled, the rest of the
    ///   selection stays, and the pressed row becomes the anchor;
    /// * `Alt` is not bound to a list gesture, so it presses like no
    ///   modifier at all.
    ///
    /// A press on the **empty space** below the last entry clears the
    /// selection (and the anchor with it); a modified press there has no row
    /// to work on and changes nothing.
    pub fn mouse_down(&mut self, x: i32, y: i32, modifiers: Modifiers) -> Option<Command> {
        self.cancel_drag();
        if self.windowshade {
            return None;
        }
        let row = self.row_at(x, y)?;
        let Some(id) = self.entries.get(self.first_visible + row).copied() else {
            // The empty space below the last entry: only the plain click
            // clears the selection there.
            if modifiers.shift() || modifiers.ctrl() {
                return None;
            }
            return self.clear_selection();
        };
        let command = if modifiers.shift() {
            self.extend_selection_to(id)
        } else if modifiers.ctrl() {
            self.toggle_row(id)
        } else {
            self.select_row(id)
        };
        self.drag = Some(PlaylistDrag {
            press: (x, y),
            ids: self.selected_ids(),
            insertion: None,
        });
        command
    }

    /// The plain click (S06 §2, settled: one posted click selects the first
    /// visible row): the pressed row alone replaces the selection and becomes
    /// the anchor of the range gestures.
    ///
    /// A row that is **already** selected keeps the multi-selection: the
    /// press arms a drag that moves the whole block (own choice, S06 §8 Q6),
    /// so it emits no command and changes nothing.
    fn select_row(&mut self, id: EntryId) -> Option<Command> {
        if self.selected.contains(&id) {
            return None;
        }
        self.selected.clear();
        self.selected.insert(id);
        self.set_anchor(id);
        Some(Command::Select(SelectOp::Set(vec![id])))
    }

    /// The ctrl-click: exactly the pressed row is toggled, every other
    /// selected row stays, and the pressed row becomes the anchor (own
    /// choice; S06 §8 Q11/Q14 leave the multi-selection open).
    ///
    /// The command is a [`SelectOp::Set`] with the **resulting** set and not
    /// a [`SelectOp::Toggle`]: the app applies every `Command::Select` back
    /// onto this model (`App::apply_app` → [`PlaylistWindow::apply_select`]),
    /// and a toggle applied twice cancels itself, while a set states the
    /// outcome and survives the round trip unchanged.
    fn toggle_row(&mut self, id: EntryId) -> Option<Command> {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        self.set_anchor(id);
        Some(Command::Select(SelectOp::Set(self.selected_ids())))
    }

    /// The shift-click: the contiguous range between the anchor and the
    /// pressed row replaces the selection, and the anchor stays (own choice;
    /// the same rule as `rustamp_playlist::interact::select_range`, which
    /// replaces the selection instead of extending it).
    ///
    /// Without an anchor — nothing was clicked yet, or the anchor's entry is
    /// gone — there is no range to extend, so the gesture behaves like a
    /// plain click: the pressed row alone is selected and becomes the anchor.
    fn extend_selection_to(&mut self, id: EntryId) -> Option<Command> {
        let Some(anchor) = self.shift_anchor else {
            return self.select_row(id);
        };
        let Some(anchor_index) = self.entries.iter().position(|entry| *entry == anchor) else {
            // A stale anchor names no row: drop it and start over.
            self.shift_anchor = None;
            self.shift_focus = None;
            return self.select_row(id);
        };
        let target_index = self.entries.iter().position(|entry| *entry == id)?;
        let ids = range_ids(&self.entries, anchor_index, target_index);
        self.selected = ids.iter().copied().collect();
        self.shift_focus = Some(id);
        Some(Command::Select(SelectOp::Set(ids)))
    }

    /// A plain click into the empty space below the last entry clears the
    /// selection, and the anchor with it (own choice).
    ///
    /// Nothing is emitted when the selection was already empty: the gesture
    /// changed nothing.
    fn clear_selection(&mut self) -> Option<Command> {
        self.shift_anchor = None;
        self.shift_focus = None;
        if self.selected.is_empty() {
            return None;
        }
        self.selected.clear();
        Some(Command::Select(SelectOp::None))
    }

    /// Sets the anchor and the focus of the range gestures to `id`.
    fn set_anchor(&mut self, id: EntryId) {
        self.shift_anchor = Some(id);
        self.shift_focus = Some(id);
    }

    /// Extends the selection from the anchor by one row (own choice: S03 §2
    /// binds neither `Shift+Up` nor `Shift+Down`, and S06 §8 Q7 leaves the
    /// keyboard open).
    ///
    /// The selected range runs between the **anchor** (the fixed end) and the
    /// **focus** (the end that moves): the focus steps one row down or up and
    /// the selection becomes the contiguous range between the two, clamped at
    /// the ends of the list. `None` when there is no anchor — nothing was
    /// clicked yet, or the anchor's entry is gone — or when the step changes
    /// nothing.
    pub fn extend_selection(&mut self, down: bool) -> Option<Command> {
        let anchor = self.shift_anchor?;
        let focus = self.shift_focus.unwrap_or(anchor);
        let focus_index = self.entries.iter().position(|entry| *entry == focus)?;
        let anchor_index = self.entries.iter().position(|entry| *entry == anchor)?;
        let last = self.entries.len().saturating_sub(1);
        let target = if down {
            (focus_index + 1).min(last)
        } else {
            focus_index.saturating_sub(1)
        };
        let ids = range_ids(&self.entries, anchor_index, target);
        let target_id = self.entries[target];
        self.shift_focus = Some(target_id);
        if ids.iter().copied().collect::<BTreeSet<_>>() == self.selected {
            return None;
        }
        self.selected = ids.iter().copied().collect();
        self.reveal(target);
        Some(Command::Select(SelectOp::Set(ids)))
    }

    /// Updates an armed drag. Four pixels of motion on either axis activate
    /// it; leaving the list cancels instead of clamping or auto-scrolling.
    /// Upper/lower row halves mean insert-before/after (own choices, Q6).
    pub fn mouse_move(&mut self, x: i32, y: i32) {
        use crate::playlist_window::LIST_TOP;

        // The row geometry is read before the drag is borrowed: the pitch
        // follows the font, so it can change while a drag is armed.
        let pitch = self.row_pitch();
        let rows = self.visible_rows();
        if self.windowshade || !self.contains_list_point(x, y) {
            self.cancel_drag();
            return;
        }
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if drag.insertion.is_none()
            && x.abs_diff(drag.press.0).max(y.abs_diff(drag.press.1)) < DRAG_THRESHOLD
        {
            return;
        }
        let relative = y - LIST_TOP;
        let row = relative / pitch;
        let after = relative % pitch >= (pitch + 1) / 2;
        let gap = (row as usize + usize::from(after)).min(rows);
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
        self.set_anchor(id);
        Some(Command::PlayEntry(id))
    }

    /// A double-click on entry `index` in the list (0-based) plays that entry.
    pub fn double_click_index(&mut self, index: usize) -> Option<Command> {
        let id = self.entries.get(index).copied()?;
        self.selected.clear();
        self.selected.insert(id);
        self.set_anchor(id);
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
    ///
    /// The anchor of the range gestures follows the moved row, so a later
    /// `Shift+Up`/`Shift+Down` extends from where the user is (own choice).
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
        self.set_anchor(id);
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
    ///
    /// The anchor follows the row: a plain `Up`/`Down` moves the user's
    /// focus, so the range gestures extend from where the user is (own
    /// choice).
    fn select_single_and_reveal(&mut self, index: usize) {
        let Some(id) = self.entries.get(index).copied() else {
            return;
        };
        self.selected.clear();
        self.selected.insert(id);
        self.set_anchor(id);
        self.reveal(index);
    }

    /// Scrolls so the row at `index` is visible.
    fn reveal(&mut self, index: usize) {
        if index < self.first_visible {
            self.scroll_to(index);
        } else if index >= self.first_visible + self.visible_rows() {
            self.scroll_to(index + 1 - self.visible_rows());
        }
    }

    /// REM removes the selected entries (S06 §5, settled on both the real and
    /// the posted path: 3475 px, same bounds (12,23)-(254,93)).
    ///
    /// The removal is a normal [`Command::Remove`]: no `#32768` window is
    /// present at capture time and no menu choice is required (S06 §5.2). With
    /// nothing selected the button is a no-op returning `None` (S06 §5.3: R2's
    /// unstaged REM second click changed 0 px).
    ///
    /// The anchor of the range gestures is dropped with its entry, exactly as
    /// [`PlaylistWindow::set_entries`] does for an entry that leaves the list.
    pub fn press_rem(&mut self) -> Option<Command> {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return None;
        }
        let gone: BTreeSet<EntryId> = ids.iter().copied().collect();
        self.entries.retain(|id| !gone.contains(id));
        self.selected.clear();
        if self
            .shift_anchor
            .is_some_and(|anchor| gone.contains(&anchor))
        {
            self.shift_anchor = None;
            self.shift_focus = None;
        } else if self.shift_focus.is_some_and(|focus| gone.contains(&focus)) {
            self.shift_focus = None;
        }
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

    /// One scrollbar-arrow step scrolls one page (S06 §3, settled as a pixel
    /// symmetry: 1501 px each way, returning on the reverse arrow). The page
    /// is as many rows as the list shows at the current font size. Clamped at
    /// both ends of the list.
    pub fn scroll_page(&mut self, direction: ScrollDirection) {
        let max_first = self.max_first_visible();
        let step = self.visible_rows();
        match direction {
            ScrollDirection::Up => {
                self.first_visible = self.first_visible.saturating_sub(step);
            }
            ScrollDirection::Down => {
                self.first_visible = (self.first_visible + step).min(max_first);
            }
        }
    }

    /// A press on the scrollbar at `(x, y)`.
    ///
    /// The scrollbar answers three parts, each with its own rule:
    ///
    /// * the **arrow pair** pages the view one page per press (S06 §3,
    ///   settled);
    /// * the **handle** takes hold of it: the drag capture starts and the
    ///   view does not move yet. A dragged handle moves the view, not a
    ///   selection (S06 §3);
    /// * the **track** pages the view one page up (above the handle) or one
    ///   page down (below it) - **own choice**: S06 §3 settles the arrows
    ///   and the handle drag and says nothing about the track. It is the
    ///   usual track behaviour and it uses the same one-page step the arrows
    ///   use.
    ///
    /// Returns whether the scrollbar took the press. The view is the model's
    /// own state, so a scrollbar press produces no command.
    pub fn press_scrollbar(&mut self, x: i32, y: i32) -> bool {
        if self.windowshade {
            return false;
        }
        let (width, height) = self.size();
        let scroll_x = width as i32 - 15;
        let arrows_y = height as i32 - 38;
        if !(scroll_x..scroll_x + 8).contains(&x) || !(20..arrows_y + 10).contains(&y) {
            return false;
        }
        if y >= arrows_y {
            self.scroll_page(if y < arrows_y + 5 {
                ScrollDirection::Up
            } else {
                ScrollDirection::Down
            });
        } else {
            let top = self.scrollbar_handle_y();
            if (top..top + 18).contains(&y) {
                self.scrollbar_grab = Some(y - top);
            } else {
                self.scroll_page(if y < top {
                    ScrollDirection::Up
                } else {
                    ScrollDirection::Down
                });
            }
        }
        true
    }

    /// Top of the scroll handle within the current client size.
    pub fn scrollbar_handle_y(&self) -> i32 {
        let travel = self.size().1 as usize - 76;
        let max_first = self.len().saturating_sub(self.visible_rows());
        20 + (travel * self.first_visible.min(max_first) + max_first / 2)
            .checked_div(max_first)
            .unwrap_or(0) as i32
    }

    /// Moves the held scrollbar handle to the pointer at `y`.
    ///
    /// Returns whether a handle was held. The pointer's position maps to the
    /// handle's top through the grab offset, and the handle's top maps to the
    /// scroll offset through
    /// [`scroll_offset_for_handle_y`](crate::playlist_window::scroll_offset_for_handle_y),
    /// so the handle follows the pointer and the view follows the handle
    /// (S06 §3: a dragged handle moves the view, not a selection). The
    /// mapping clamps at both ends of the track and the offset is clamped
    /// into the list, so the view never leaves it.
    pub fn drag_scrollbar(&mut self, y: i32) -> bool {
        let Some(grab) = self.scrollbar_grab else {
            return false;
        };
        let top = y - grab;
        let travel = self.size().1 as usize - 76;
        let max_first = self.len().saturating_sub(self.visible_rows());
        let position = (top - 20).clamp(0, travel as i32) as usize;
        let offset = (position * max_first + travel / 2) / travel;
        self.scroll_to(offset);
        true
    }

    /// Ends the scrollbar handle drag.
    ///
    /// Returns whether a handle was held.
    pub fn release_scrollbar(&mut self) -> bool {
        self.scrollbar_grab.take().is_some()
    }

    /// Whether the scrollbar handle is held.
    pub fn scrollbar_captured(&self) -> bool {
        self.scrollbar_grab.is_some()
    }

    /// Scrolls so `index` is the first visible entry, clamped into the list.
    pub fn scroll_to(&mut self, index: usize) {
        self.first_visible = index.min(self.max_first_visible());
    }

    /// The largest scroll offset: the bottom page shows the last entries of
    /// the page. S01 §4.2 measured the bottom view of the 60-entry list as
    /// rows 57-60 with the handle at its lowest point, so the offset range
    /// is `len - 4` at the default font size, the same range
    /// [`scroll_handle_y`](crate::scroll_handle_y) assumes. A list that fits
    /// one page has offset 0.
    pub fn max_first_visible(&self) -> usize {
        self.entries.len().saturating_sub(self.visible_rows())
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
            window.mouse_down(60, 28, Modifiers::NONE),
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
        assert_eq!(window.mouse_down(60, 42, Modifiers::NONE), None);
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
        window.mouse_down(60, 28, Modifiers::NONE);
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
            window.mouse_down(60, 70, Modifiers::NONE),
            None,
            "blank space cannot start a drag"
        );
        window.mouse_down(60, 28, Modifiers::NONE);
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
        window.mouse_down(60, 28, Modifiers::NONE);
        assert_eq!(window.mouse_up(63, 28), None);
        for y in [36, 48, 49, 61] {
            window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(2)]));
            window.mouse_down(60, 42, Modifiers::NONE);
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
            window.mouse_down(60, 28, Modifiers::NONE);
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
        window.mouse_down(60, 28, Modifiers::NONE);
        window.mouse_move(60, 74);
        window.set_entries(vec![EntryId(1), EntryId(0), EntryId(2), EntryId(3)]);
        assert_eq!(window.mouse_up(60, 74), None, "concurrent reorder cancels");
        window.mouse_down(60, 28, Modifiers::NONE);
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
        let command = window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1, Modifiers::NONE);
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
            window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1, Modifiers::NONE),
            None
        );
        assert!(window.selected_ids().is_empty());
    }

    /// The window-relative y of the visible row `row`.
    ///
    /// The list starts at the settled top of S06 §2 and the rows follow the
    /// settled 13 px pitch of S01 §4.2.
    fn row_y(row: usize) -> i32 {
        crate::playlist_window::LIST_TOP + row as i32 * crate::playlist_window::ROW_PITCH
    }

    /// The shift-click selects the contiguous range between the anchor and the
    /// pressed row, in both directions (own choice).
    ///
    /// S06 §2 settles only the single click and SEL; S06 §8 Q11/Q14 leave the
    /// multi-selection open. The rule mirrors `rustamp_playlist::interact`'s
    /// `select_range`: a contiguous range that **replaces** the selection.
    #[test]
    fn a_shift_click_selects_the_contiguous_range_from_the_anchor() {
        let mut window = window_with(6);
        assert_eq!(
            window.mouse_down(60, row_y(1), Modifiers::NONE),
            Some(Command::Select(SelectOp::Set(vec![EntryId(1)])))
        );
        // Forward: the anchor stays, the range reaches the pressed row.
        assert_eq!(
            window.mouse_down(60, row_y(3), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(vec![
                EntryId(1),
                EntryId(2),
                EntryId(3)
            ])))
        );
        assert_eq!(
            window.selected_ids(),
            vec![EntryId(1), EntryId(2), EntryId(3)]
        );
        // Backward: the same anchor, the range between anchor and pressed row.
        assert_eq!(
            window.mouse_down(60, row_y(0), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(vec![EntryId(0), EntryId(1)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(0), EntryId(1)]);
    }

    /// The shift-range is clamped at both ends of the list (own choice).
    #[test]
    fn a_shift_click_is_clamped_at_both_ends_of_the_list() {
        // The anchor is the last row: the range reaches the top of the list.
        let mut window = window_with(4);
        window.mouse_down(60, row_y(3), Modifiers::NONE);
        assert_eq!(
            window.mouse_down(60, row_y(0), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(ids(4))))
        );
        assert_eq!(window.selected_ids(), ids(4));
        // The anchor is the first row: the range reaches the bottom.
        let mut window = window_with(4);
        window.mouse_down(60, row_y(0), Modifiers::NONE);
        assert_eq!(
            window.mouse_down(60, row_y(3), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(ids(4))))
        );
        assert_eq!(window.selected_ids(), ids(4));
    }

    /// The ctrl-click toggles exactly the pressed row and leaves the rest of
    /// the selection alone (own choice).
    #[test]
    fn a_ctrl_click_toggles_exactly_the_pressed_row() {
        let mut window = window_with(4);
        window.mouse_down(60, row_y(0), Modifiers::NONE);
        // + row 2
        assert_eq!(
            window.mouse_down(60, row_y(2), Modifiers::CTRL),
            Some(Command::Select(SelectOp::Set(vec![EntryId(0), EntryId(2)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(0), EntryId(2)]);
        // - row 0: the other row stays selected.
        assert_eq!(
            window.mouse_down(60, row_y(0), Modifiers::CTRL),
            Some(Command::Select(SelectOp::Set(vec![EntryId(2)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(2)]);
        // - row 2: the selection is empty, and the anchor is the pressed row.
        assert_eq!(
            window.mouse_down(60, row_y(2), Modifiers::CTRL),
            Some(Command::Select(SelectOp::Set(Vec::new())))
        );
        assert!(window.selected_ids().is_empty());
    }

    /// The shift-arrows extend the selection from the anchor by one row
    /// (own choice: S03 §2 binds neither chord, S06 §8 Q7 leaves the keyboard
    /// open).
    #[test]
    fn the_shift_arrows_extend_the_selection_from_the_anchor() {
        let mut window = window_with(8);
        window.mouse_down(60, row_y(1), Modifiers::NONE);
        // One row down: the range spans the anchor and the focus.
        assert_eq!(
            window.extend_selection(true),
            Some(Command::Select(SelectOp::Set(vec![EntryId(1), EntryId(2)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(1), EntryId(2)]);
        // One row more.
        assert_eq!(
            window.extend_selection(true),
            Some(Command::Select(SelectOp::Set(vec![
                EntryId(1),
                EntryId(2),
                EntryId(3)
            ])))
        );
        // Back up: the range shrinks towards the anchor.
        assert_eq!(
            window.extend_selection(false),
            Some(Command::Select(SelectOp::Set(vec![EntryId(1), EntryId(2)])))
        );
        // Up over the anchor: the range grows above it.
        assert_eq!(
            window.extend_selection(false),
            Some(Command::Select(SelectOp::Set(vec![EntryId(1)])))
        );
        assert_eq!(
            window.extend_selection(false),
            Some(Command::Select(SelectOp::Set(vec![EntryId(0), EntryId(1)])))
        );
        // Clamped at the top of the list: nothing changes, nothing is emitted.
        assert_eq!(window.extend_selection(false), None);
        assert_eq!(window.selected_ids(), vec![EntryId(0), EntryId(1)]);
        // Clamped at the bottom of the list.
        let mut window = window_with(4);
        window.mouse_down(60, row_y(3), Modifiers::NONE);
        assert_eq!(window.extend_selection(true), None);
        assert_eq!(window.selected_ids(), vec![EntryId(3)]);
    }

    /// The anchor is an *entry*, so a reorder moves its row with it (own
    /// choice; the same rule as `rustamp_playlist::interact`, whose caller
    /// re-derives the anchor's row after a reorder).
    #[test]
    fn the_anchor_survives_a_reorder() {
        let mut window = window_with(5);
        // The anchor is the entry 2 (row 2).
        window.mouse_down(60, row_y(2), Modifiers::NONE);
        window.set_entries(vec![
            EntryId(0),
            EntryId(1),
            EntryId(4),
            EntryId(2),
            EntryId(3),
        ]);
        // The anchor (entry 2) is row 3 now: the range runs from row 3 back
        // to the pressed row 1.
        assert_eq!(
            window.mouse_down(60, row_y(1), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(vec![
                EntryId(1),
                EntryId(4),
                EntryId(2),
            ])))
        );
        assert_eq!(
            window.selected_ids(),
            vec![EntryId(1), EntryId(4), EntryId(2)]
        );
    }

    /// An anchor whose entry is gone names no row, so the gesture must not
    /// select one (own choice; the same rule as `rustamp_playlist::interact`).
    #[test]
    fn a_removed_anchor_never_selects_a_wrong_row() {
        let mut window = window_with(5);
        window.mouse_down(60, row_y(2), Modifiers::NONE);
        // The anchor's entry leaves the list: the anchor is dropped with it.
        window.set_entries(vec![EntryId(0), EntryId(1), EntryId(3), EntryId(4)]);
        assert!(window.selected_ids().is_empty());
        // A shift-click without an anchor has no range to extend: it selects
        // the pressed row alone.
        assert_eq!(
            window.mouse_down(60, row_y(3), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(vec![EntryId(4)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(4)]);
        // The keyboard extension refuses without an anchor.
        let mut window = PlaylistWindow::new();
        window.set_entries(ids(4));
        assert_eq!(window.extend_selection(true), None);
        assert!(window.selected_ids().is_empty());
    }

    /// An empty list refuses every gesture (own choice; the same rule as
    /// `rustamp_playlist::interact`).
    #[test]
    fn an_empty_list_refuses_every_selection_gesture() {
        let mut window = PlaylistWindow::new();
        for modifiers in [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::CTRL,
            Modifiers::SHIFT.union(Modifiers::CTRL),
        ] {
            assert_eq!(
                window.mouse_down(60, row_y(0), modifiers),
                None,
                "{modifiers:?}"
            );
        }
        assert_eq!(window.extend_selection(true), None);
        assert_eq!(window.extend_selection(false), None);
        assert!(window.selected_ids().is_empty());
    }

    /// A plain click into the empty space below the last entry clears the
    /// selection, and the anchor with it (own choice).
    #[test]
    fn a_plain_click_into_the_empty_space_clears_the_selection_and_the_anchor() {
        let mut window = window_with(2);
        window.mouse_down(60, row_y(0), Modifiers::NONE);
        assert_eq!(window.selected_ids(), vec![EntryId(0)]);
        // Row 2 is empty space: the list holds two entries.
        assert_eq!(
            window.mouse_down(60, row_y(2), Modifiers::NONE),
            Some(Command::Select(SelectOp::None))
        );
        assert!(window.selected_ids().is_empty());
        // The anchor is gone with it: the shift-click has no range to extend.
        assert_eq!(
            window.mouse_down(60, row_y(1), Modifiers::SHIFT),
            Some(Command::Select(SelectOp::Set(vec![EntryId(1)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(1)]);
    }

    /// A modified click into the empty space has no row to work on and
    /// changes nothing (own choice).
    #[test]
    fn a_modified_click_into_the_empty_space_changes_nothing() {
        let mut window = window_with(2);
        window.mouse_down(60, row_y(0), Modifiers::NONE);
        assert_eq!(window.mouse_down(60, row_y(2), Modifiers::SHIFT), None);
        assert_eq!(window.mouse_down(60, row_y(2), Modifiers::CTRL), None);
        assert_eq!(window.selected_ids(), vec![EntryId(0)]);
    }

    /// The settled single click still replaces the selection with one row
    /// (S06 §2): the gestures around it do not change that.
    #[test]
    fn the_plain_click_still_replaces_the_selection_with_one_row() {
        let mut window = window_with(4);
        window.mouse_down(60, row_y(0), Modifiers::NONE);
        window.mouse_down(60, row_y(2), Modifiers::CTRL);
        assert_eq!(window.selected_ids(), vec![EntryId(0), EntryId(2)]);
        assert_eq!(
            window.mouse_down(60, row_y(3), Modifiers::NONE),
            Some(Command::Select(SelectOp::Set(vec![EntryId(3)])))
        );
        assert_eq!(window.selected_ids(), vec![EntryId(3)]);
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
        window.mouse_down(LIST_SELECT_POINT.0, LIST_SELECT_POINT.1, Modifiers::NONE);
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
            crate::playlist_window::scroll_handle_y(
                window.first_visible(),
                window.len(),
                window.row_pitch()
            ),
            60
        );
    }

    /// S06 §3: a dragged handle **moves the view, not a selection**.
    ///
    /// The handle's positions are settled (S01 §4.2: y = 20 at the scroll top
    /// and y = 60 at the bottom of a 60-entry list), so the pixel-to-row
    /// mapping is this implementation's own choice - the one
    /// [`scroll_offset_for_handle_y`](crate::playlist_window::scroll_offset_for_handle_y)
    /// owns. What the spec settles is the effect: the view follows the
    /// handle and the selection stays where it was.
    #[test]
    fn a_dragged_handle_moves_the_view_and_not_the_selection() {
        let mut window = window_with(60);
        window.apply_select(&SelectOp::Set(vec![EntryId(0)]));
        // At the scroll top the handle sits at the top of the track (y = 20).
        assert_eq!(
            crate::playlist_window::scroll_handle_y(0, 60, window.row_pitch()),
            20
        );
        // A press on the handle takes hold of it; the view does not move yet.
        assert!(window.press_scrollbar(263, 20));
        assert!(window.scrollbar_captured());
        assert_eq!(window.first_visible(), 0);
        // The drag to the bottom of the track maps through the model's helper.
        window.drag_scrollbar(60);
        assert_eq!(
            window.first_visible(),
            crate::playlist_window::scroll_offset_for_handle_y(60, 60, window.row_pitch())
        );
        assert_eq!(window.first_visible(), 56, "the bottom page of 60 entries");
        // The view moved ...
        assert_eq!(window.visible_ids()[0], EntryId(56));
        // ... and the selection did not move with it.
        assert_eq!(window.selected_ids(), vec![EntryId(0)]);
        // The release ends the capture; a later move changes nothing.
        assert!(window.release_scrollbar());
        assert!(!window.scrollbar_captured());
        window.drag_scrollbar(20);
        assert_eq!(window.first_visible(), 56);
        assert!(!window.release_scrollbar(), "the capture was already gone");
    }

    /// The drag clamps at both ends of the track: the view never leaves the
    /// list, whatever the pointer does.
    #[test]
    fn the_handle_drag_clamps_at_both_ends_of_the_track() {
        let mut window = window_with(60);
        assert!(window.press_scrollbar(263, 20));
        // Far above the track: the view is at the top of the list.
        window.drag_scrollbar(-500);
        assert_eq!(window.first_visible(), 0);
        // Far below the track: the view is at the bottom page, and no further.
        window.drag_scrollbar(5_000);
        assert_eq!(window.first_visible(), 56);
        // The offset never leaves the list's range.
        for y in -100..200 {
            window.drag_scrollbar(y);
            assert!(window.first_visible() <= window.max_first_visible());
        }
    }

    /// Taking hold of the handle does not move the view: the handle keeps
    /// the place the pointer took hold of it (own choice - the spec settles
    /// that a dragged handle moves the view, not how the grab behaves).
    #[test]
    fn taking_hold_of_the_handle_keeps_the_grab_and_the_view() {
        let mut window = window_with(60);
        window.scroll_to(56);
        // At the bottom page the handle is at y = 60..77; the pointer takes
        // hold of it eight pixels below its top.
        assert_eq!(
            crate::playlist_window::scroll_handle_y(56, 60, window.row_pitch()),
            60
        );
        assert!(window.press_scrollbar(263, 68));
        assert_eq!(
            window.first_visible(),
            56,
            "the press did not move the view"
        );
        // The handle follows the pointer: eight pixels below the pointer, its
        // top reaches the top of the track.
        window.drag_scrollbar(28);
        assert_eq!(window.first_visible(), 0, "the handle's top reached y = 20");
    }

    /// S06 §3's visual reading of the handle drag lines up with the settled
    /// handle positions.
    ///
    /// The spec quotes the rows `44 ref`, `45 tl`, `47` for a drag from
    /// (263,20) to (263,60) - the last page of a 48-entry list. The handle's
    /// positions are settled (S01 §4.2: y = 20 at the scroll top, y = 60 at
    /// the bottom), so a drag to the bottom of the track shows the bottom
    /// page. The spec's own phrase ("moves the view toward the top of the
    /// list") does not match the rows it quotes; the rows match the settled
    /// positions, and this mapping reproduces them.
    #[test]
    fn the_handle_drag_reproduces_the_rows_s06_quotes() {
        let mut window = window_with(48);
        assert!(window.press_scrollbar(263, 20));
        window.drag_scrollbar(60);
        assert_eq!(window.first_visible(), 44);
        let visible = window.visible_ids();
        assert_eq!(visible[0], EntryId(44));
        assert_eq!(visible[3], EntryId(47));
    }

    /// A press on the track pages the view.
    ///
    /// **Own choice**: S06 §3 settles the arrow points and the handle drag
    /// and says nothing about the track. This is the usual track behaviour,
    /// and it uses the same one-page step the arrows use.
    #[test]
    fn a_press_on_the_track_pages_the_view() {
        let mut window = window_with(60);
        window.scroll_to(28);
        // In the middle of the list the handle is in the middle of the track
        // (y = 40..57).
        assert_eq!(
            crate::playlist_window::scroll_handle_y(28, 60, window.row_pitch()),
            40
        );
        // Above the handle: one page up.
        assert!(window.press_scrollbar(263, 25));
        assert_eq!(window.first_visible(), 24);
        // Below the handle: one page down.
        assert!(window.press_scrollbar(263, 70));
        assert_eq!(window.first_visible(), 28);
    }

    /// The arrow pair keeps its settled one-page step (S06 §3) through the
    /// same press, and a press outside the scrollbar is not the scrollbar's.
    #[test]
    fn the_arrow_pair_pages_the_view_through_the_same_press() {
        let mut window = window_with(60);
        assert!(window.press_scrollbar(263, 87));
        assert_eq!(window.first_visible(), 4);
        assert!(window.press_scrollbar(263, 81));
        assert_eq!(window.first_visible(), 0);
        // Outside the scrollbar column nothing hits.
        assert!(!window.press_scrollbar(259, 30));
        assert!(!window.press_scrollbar(268, 30));
        assert!(!window.press_scrollbar(60, 30));
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
        // Ctrl+E is the playlist's own row (`40255` "Edit the playlist
        // entry"); it never reaches the main table's `40186` "Toggle easy
        // move".
        assert_eq!(
            playlist_key_action(Key::Letter(b'E'), Modifiers::CTRL),
            Some(PlaylistKeyAction::EditEntry)
        );
        assert_eq!(
            unimplemented_playlist_row(Key::Letter(b'E'), Modifiers::CTRL),
            None
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
        // Home, End, PgUp, PgDn, F3, Ctrl+F4, F5, Alt+Enter, Ctrl+Tab, and
        // the two font-size rows.
        assert_eq!(provisional.len(), 20);
        for id in [
            40043, 40334, 40305, 40059, 40058, 40290, 40216, 40289, 40215, 40264, 40265, 40256,
            40257, 40318, 40224, 40291, 40337, 40063, 40335, 40336,
        ] {
            assert!(provisional.contains(&id), "{id}");
        }
        // The two font-size rows are bound rows of the table now, and they are
        // provisional (S03 §2 marks them so).
        for id in [40335, 40336] {
            assert!(
                PLAYLIST_TABLE
                    .iter()
                    .any(|binding| binding.action.command_id() == id && binding.provisional),
                "{id}"
            );
            assert!(
                !UNIMPLEMENTED_PLAYLIST_ROWS
                    .iter()
                    .any(|row| row.command_id == id),
                "{id} must not be listed as unimplemented"
            );
        }
    }

    #[test]
    fn the_unimplemented_rows_are_the_s03_rows_without_a_command() {
        let rows: Vec<u32> = UNIMPLEMENTED_PLAYLIST_ROWS
            .iter()
            .map(|row| row.command_id)
            .collect();
        // Every row of S03 §2 has an action now: `Ctrl+E` (`40255` "Edit the
        // playlist entry") was the last gap, and it is a bound row of the
        // table. `Ctrl+Alt+E` (`40293`) reads the extended info of the
        // selection and is a bound row as well.
        assert!(rows.is_empty(), "{rows:?}");
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

    /// S03 §2's `Ctrl+E` ("Edit the playlist entry", `40255`) is a bound row:
    /// it opens the edit-entry dialog, whose model is
    /// `rustamp_ui::dialogs::EditEntryDialog`.
    #[test]
    fn the_edit_entry_row_is_bound_and_no_longer_unimplemented() {
        assert_eq!(
            playlist_key_action(Key::Letter(b'E'), Modifiers::CTRL),
            Some(PlaylistKeyAction::EditEntry)
        );
        assert_eq!(PlaylistKeyAction::EditEntry.command_id(), 40255);
        assert_eq!(
            unimplemented_playlist_row(Key::Letter(b'E'), Modifiers::CTRL),
            None,
            "the row is implemented, so it is not listed as unimplemented"
        );
        // The row is not provisional: S03 §2 confirms it through the menu.
        let binding =
            playlist_key_binding(Key::Letter(b'E'), Modifiers::CTRL).expect("the row is bound");
        assert!(!binding.provisional);
    }

    /// S03 §2 `Ctrl+Alt+E` ("Read extended info on selection", `40293`) is a
    /// bound row: it reads the tags of the selected entries and shows them.
    #[test]
    fn the_extended_info_row_is_bound_and_no_longer_unimplemented() {
        let chord = Modifiers::CTRL.union(Modifiers::ALT);
        assert_eq!(
            playlist_key_action(Key::Letter(b'E'), chord),
            Some(PlaylistKeyAction::ReadExtendedInfo)
        );
        assert_eq!(PlaylistKeyAction::ReadExtendedInfo.command_id(), 40293);
        assert_eq!(
            unimplemented_playlist_row(Key::Letter(b'E'), chord),
            None,
            "the row is implemented, so it is not listed as unimplemented"
        );
        // The row is not provisional: S03 §2 confirms it through the menu.
        let binding = playlist_key_binding(Key::Letter(b'E'), chord).expect("the row is bound");
        assert!(!binding.provisional);
    }

    /// S03 §2's two font-size rows are bound and act on the playlist machine
    /// alone: they change the rows the editor draws and nothing else, so
    /// they need no core command (RFC 0005 §9/§11.6).
    #[test]
    fn the_font_size_rows_are_bound_and_walk_the_size_within_its_range() {
        assert_eq!(
            playlist_key_action(Key::NumAdd, Modifiers::CTRL),
            Some(PlaylistKeyAction::IncreaseFontSize)
        );
        assert_eq!(
            playlist_key_action(Key::NumSubtract, Modifiers::CTRL),
            Some(PlaylistKeyAction::DecreaseFontSize)
        );
        assert_eq!(
            playlist_key_action(Key::NumAdd, Modifiers::NONE),
            None,
            "the row needs Ctrl"
        );
        assert_eq!(
            unimplemented_playlist_row(Key::NumAdd, Modifiers::CTRL),
            None
        );

        let mut window = window_with(4);
        assert_eq!(window.font_size().size(), 13, "the default size");
        assert!(!window.font_size().is_overridden());
        assert!(window.increase_font_size());
        assert_eq!(window.font_size().size(), 14);
        assert!(window.font_size().is_overridden());
        assert!(window.decrease_font_size());
        assert_eq!(window.font_size().size(), 13);
        // The size is clamped at both ends of the range.
        for _ in 0..1000 {
            window.increase_font_size();
        }
        assert_eq!(
            window.font_size().size(),
            crate::playlist_font::MAX_FONT_SIZE
        );
        assert!(!window.increase_font_size(), "the largest size is the end");
        for _ in 0..1000 {
            window.decrease_font_size();
        }
        assert_eq!(
            window.font_size().size(),
            crate::playlist_font::MIN_FONT_SIZE
        );
        assert!(!window.decrease_font_size(), "the smallest size is the end");
    }

    /// The row height follows the font, so the page holds more rows with a
    /// smaller font and fewer with a bigger one - and no row is clipped.
    #[test]
    fn the_row_count_follows_the_font_size() {
        let mut window = window_with(60);
        assert_eq!(window.row_pitch(), 13);
        assert_eq!(window.visible_rows(), 4);
        assert_eq!(window.visible_ids().len(), 4);

        // The skin's size is used until the user changes it.
        window.set_skin_font_size(10);
        assert_eq!(window.row_pitch(), 10);
        assert_eq!(window.visible_rows(), 5);
        assert_eq!(window.visible_ids().len(), 5);

        // The user's size wins over the skin's from then on.
        window.increase_font_size();
        assert_eq!(window.row_pitch(), 11);
        assert_eq!(window.visible_rows(), 4);
        window.set_skin_font_size(20);
        assert_eq!(
            window.row_pitch(),
            11,
            "the user's size wins over the skin's"
        );

        // A bigger font: fewer rows, and they still fit the page.
        window.set_font_size_override(Some(26));
        assert_eq!(window.row_pitch(), 26);
        assert_eq!(window.visible_rows(), 2);
        assert_eq!(window.visible_ids().len(), 2);
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
