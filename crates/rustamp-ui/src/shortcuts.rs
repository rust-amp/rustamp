// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main window's keyboard shortcuts (spec **S03** §1, §4 and §6).
//!
//! Everything here is headless: there is no `winit`, no `softbuffer` and no
//! clock. [`Key`] and [`Modifiers`] describe a key press independently of any
//! windowing library; the presenter converts `winit` events into them.
//!
//! * [`MAIN_TABLE`] is the S03 §1 table, one [`Binding`] per row (62 rows).
//! * [`GLOBAL_TABLE`] is the S03 §4 table (29 rows).
//! * [`lookup`] resolves a key press to a [`ShortcutAction`].
//! * [`MainWindow::key_down`] dispatches the action and returns a
//!   [`ShortcutOutcome`].
//!
//! # Provisional
//!
//! * **Table combination** (S03 §4, §6.1): how the tables combine is
//!   provisional. [`lookup`] checks the window's table first, then the global
//!   table; on a conflict (`Ctrl+L`) the window's table wins.
//! * **Rows S03 marks provisional** carry [`Binding::provisional`] `== true`.
//!   `Ctrl+W` is not among them: S03 §1 records the command as observed.
//! * **Volume step**: see [`VOLUME_STEP`].
//! * **Key repeat**: a repeated key press delivers its action again; the
//!   dispatch does not distinguish a repeat from a first press. This is a
//!   choice of this implementation, not an S03 fact.

use rustamp_core::command::Command;
use rustamp_core::state::WindowId;
// `rustamp-core` re-exports the RFC 0005 `Command` *variants* through `Command`
// automatically, but its `command` module re-exports an explicit item list, so
// the three new *types* are not reachable there. They come straight from the
// permissive `rustamp-proto` instead of editing `rustamp-core` (a contract
// change). Reported as a finding.
use rustamp_proto::command::{JumpTarget, OpenMediaKind, PrefsPage};

use crate::behaviour::{MainWindow, core_volume};

/// The seek step of the `Left`/`Right`/`Num7`/`Num9` keys in milliseconds
/// (S03 §1: "Rewind 5 seconds" / "Forward 5 seconds").
pub const SEEK_STEP_MS: i64 = 5000;

/// The step of the `Num1`/`Num3` keys ("10 tracks back" / "10 tracks
/// forward", S03 §1), carried on the wire as `Command::SkipBy { delta }`
/// (RFC 0005 §5.1: the step stays in the tables that already carry the ids, so
/// the engine has one generic code path).
pub const TEN_TRACK_STEP: i32 = 10;

/// The volume change of one `Up`/`Down`/`Num8`/`Num2` key press, in core
/// volume units (`0..=255`).
///
/// **Provenance (measured, 2026-10-15):** posting `40058`/`40059` to the
/// reference 2.95 player moves its volume in steps of **4** core units
/// (`0 -> 4 -> 8 ...`), saturating at `0`/`255` with partial end steps
/// (`253 -> 255`, `2 -> 0`); cross-checked through the volume write path. The
/// measurement memo is `notes/2026-10-15-wm-command-o5-o6-o7-o10-o15.md`
/// (dirty room) and RFC 0005 §8 (O10 settled). The keys themselves stay
/// provisional rows of S03 §1, and the step never crosses IPC: the wire carries
/// the absolute [`Command::SetVolume`] only.
pub const VOLUME_STEP: u8 = 4;

/// A key, independent of any windowing library.
///
/// Letters are stored as upper-case ASCII; [`Key::letter`] normalizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter key, upper-case ASCII `A..=Z`.
    Letter(u8),
    /// A top-row digit `0..=9`.
    Digit(u8),
    /// A function key `F1..=F24`.
    F(u8),
    /// A numeric-keypad digit `Num0..=Num9`.
    Numpad(u8),
    /// The keypad decimal key (`Num.`).
    NumDecimal,
    /// The keypad plus key (`Num+`).
    NumAdd,
    /// The keypad minus key (`Num-`).
    NumSubtract,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Enter / Return.
    Enter,
    /// Tab.
    Tab,
    /// The grave (backtick) key.
    Grave,
    /// Space.
    Space,
    /// Escape.
    Escape,
}

impl Key {
    /// A letter key from an ASCII character (either case); `None` otherwise.
    pub fn letter(c: char) -> Option<Self> {
        c.is_ascii_alphabetic()
            .then(|| Self::Letter(c.to_ascii_uppercase() as u8))
    }
}

/// A set of modifier keys (Ctrl, Alt, Shift).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier.
    pub const NONE: Self = Self(0);
    /// Ctrl.
    pub const CTRL: Self = Self(1);
    /// Alt.
    pub const ALT: Self = Self(2);
    /// Shift.
    pub const SHIFT: Self = Self(4);

    /// Builds a set from three flags.
    pub const fn new(ctrl: bool, alt: bool, shift: bool) -> Self {
        Self((ctrl as u8) | ((alt as u8) << 1) | ((shift as u8) << 2))
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether Ctrl is in the set.
    pub const fn ctrl(self) -> bool {
        self.0 & 1 != 0
    }

    /// Whether Alt is in the set.
    pub const fn alt(self) -> bool {
        self.0 & 2 != 0
    }

    /// Whether Shift is in the set.
    pub const fn shift(self) -> bool {
        self.0 & 4 != 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

/// Which window's table applies (S03 §1-§3).
///
/// Only the main window's table is filled; the playlist editor (S03 §2) and
/// the equalizer (S03 §3) do not exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShortcutWindow {
    /// The main window (S03 §1).
    Main,
}

/// One distinct action of S03 §1 / §4, carrying its public `WM_COMMAND` id.
///
/// The ids are interoperability facts recorded in S03.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    /// Preferences (command id 40012).
    Preferences,
    /// Toggle always on top (command id 40019).
    ToggleAlwaysOnTop,
    /// Toggle repeat (command id 40022).
    ToggleRepeat,
    /// Toggle shuffle (command id 40023).
    ToggleShuffle,
    /// Open file(s) (command id 40029).
    OpenFiles,
    /// Toggle equalizer (command id 40036).
    ToggleEqualizer,
    /// Toggle playlist editor (command id 40040).
    TogglePlaylistEditor,
    /// About box (command id 40041).
    About,
    /// Open the main menu (command id 40043).
    OpenMainMenu,
    /// Previous track (command id 40044).
    PreviousTrack,
    /// Play (command id 40045).
    Play,
    /// Pause / unpause (command id 40046).
    PauseUnpause,
    /// Stop (command id 40047).
    Stop,
    /// Next track (command id 40048).
    NextTrack,
    /// Volume up (command id 40058).
    VolumeUp,
    /// Volume down (command id 40059).
    VolumeDown,
    /// Forward 5 seconds (command id 40060).
    Forward5s,
    /// Rewind 5 seconds (command id 40061).
    Rewind5s,
    /// Switch to the next window (command id 40063).
    NextWindow,
    /// Toggle windowshade (command id 40064).
    ToggleWindowShade,
    /// Stop with fadeout (command id 40147).
    StopWithFadeout,
    /// Start of list (command id 40154).
    StartOfList,
    /// Stop after current track (command id 40157).
    StopAfterCurrent,
    /// Toggle double size (command id 40165).
    ToggleDoubleSize,
    /// Open location (command id 40185).
    OpenLocation,
    /// Toggle easy move (command id 40186).
    ToggleEasyMove,
    /// Open directory (command id 40187).
    OpenDirectory,
    /// View file info (command id 40188).
    ViewFileInfo,
    /// Select visualization plug-in (command id 40191).
    SelectVisPlugin,
    /// Start/stop visualization plug-in (command id 40192).
    ToggleVisPlugin,
    /// Jump to time (command id 40193).
    JumpToTime,
    /// Jump to file (command id 40194).
    JumpToFile,
    /// 10 tracks forward (command id 40195).
    TenTracksForward,
    /// 10 tracks back (command id 40197).
    TenTracksBack,
    /// Skin browser (command id 40219).
    SkinBrowser,
    /// Configure visualization plug-in (command id 40221).
    ConfigureVisPlugin,
    /// Close the focused window (command id 40224).
    CloseFocusedWindow,
    /// Toggle elapsed/remaining time (command id 40242).
    ToggleTimeRemaining,
    /// Show the main window (command id 40258).
    ShowMainWindow,
    /// Refresh skin (command id 40291).
    RefreshSkin,
    /// Toggle minibrowser (command id 40298).
    ToggleMinibrowser,
    /// Minibrowser command (unconfirmed) (command id 40299).
    MinibrowserAltRight,
    /// Minibrowser command (unconfirmed) (command id 40300).
    MinibrowserAltLeft,
    /// Minibrowser command (unconfirmed) (command id 40301).
    MinibrowserCtrlR,
    /// Minibrowser command (unconfirmed) (command id 40302).
    MinibrowserCtrlL,
    /// Minibrowser command (unconfirmed) (command id 40303).
    MinibrowserCtrlO,
    /// Start a new instance (command id 40305).
    NewInstance,
    /// Minibrowser command (unconfirmed) (command id 40309).
    MinibrowserCtrlAltR,
    /// Visualization options (command id 40317).
    VisOptions,
    /// Playlist search (unconfirmed) (command id 40318).
    PlaylistSearch,
    /// Edit bookmarks (command id 40320).
    EditBookmarks,
    /// Add current track as a bookmark (command id 40321).
    AddBookmark,
    /// Toggle video window (command id 40328).
    ToggleVideoWindow,
    /// Minimize (command id 40334).
    Minimize,
    /// Toggle video fullscreen (command id 40337).
    ToggleVideoFullscreen,
}

impl ShortcutAction {
    /// The public command id S03 records for this action.
    pub const fn command_id(self) -> u32 {
        match self {
            Self::Preferences => 40012,
            Self::ToggleAlwaysOnTop => 40019,
            Self::ToggleRepeat => 40022,
            Self::ToggleShuffle => 40023,
            Self::OpenFiles => 40029,
            Self::ToggleEqualizer => 40036,
            Self::TogglePlaylistEditor => 40040,
            Self::About => 40041,
            Self::OpenMainMenu => 40043,
            Self::PreviousTrack => 40044,
            Self::Play => 40045,
            Self::PauseUnpause => 40046,
            Self::Stop => 40047,
            Self::NextTrack => 40048,
            Self::VolumeUp => 40058,
            Self::VolumeDown => 40059,
            Self::Forward5s => 40060,
            Self::Rewind5s => 40061,
            Self::NextWindow => 40063,
            Self::ToggleWindowShade => 40064,
            Self::StopWithFadeout => 40147,
            Self::StartOfList => 40154,
            Self::StopAfterCurrent => 40157,
            Self::ToggleDoubleSize => 40165,
            Self::OpenLocation => 40185,
            Self::ToggleEasyMove => 40186,
            Self::OpenDirectory => 40187,
            Self::ViewFileInfo => 40188,
            Self::SelectVisPlugin => 40191,
            Self::ToggleVisPlugin => 40192,
            Self::JumpToTime => 40193,
            Self::JumpToFile => 40194,
            Self::TenTracksForward => 40195,
            Self::TenTracksBack => 40197,
            Self::SkinBrowser => 40219,
            Self::ConfigureVisPlugin => 40221,
            Self::CloseFocusedWindow => 40224,
            Self::ToggleTimeRemaining => 40242,
            Self::ShowMainWindow => 40258,
            Self::RefreshSkin => 40291,
            Self::ToggleMinibrowser => 40298,
            Self::MinibrowserAltRight => 40299,
            Self::MinibrowserAltLeft => 40300,
            Self::MinibrowserCtrlR => 40301,
            Self::MinibrowserCtrlL => 40302,
            Self::MinibrowserCtrlO => 40303,
            Self::NewInstance => 40305,
            Self::MinibrowserCtrlAltR => 40309,
            Self::VisOptions => 40317,
            Self::PlaylistSearch => 40318,
            Self::EditBookmarks => 40320,
            Self::AddBookmark => 40321,
            Self::ToggleVideoWindow => 40328,
            Self::Minimize => 40334,
            Self::ToggleVideoFullscreen => 40337,
        }
    }
}

/// One table row: a key press and the action it triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// The key.
    pub key: Key,
    /// The exact modifier set (no other modifier may be held).
    pub modifiers: Modifiers,
    /// The action.
    pub action: ShortcutAction,
    /// `true` when S03 marks the row provisional.
    pub provisional: bool,
}

const fn row(key: Key, modifiers: Modifiers, action: ShortcutAction, provisional: bool) -> Binding {
    Binding {
        key,
        modifiers,
        action,
        provisional,
    }
}

/// The main-window table, S03 §1: 62 rows in the spec's order.
///
/// Duplicate bindings (`Num.`, `Insert`, `X`, `Z`, `Num0`, `Num4`..`Num9`, ...)
/// are separate rows with the same action.
pub const MAIN_TABLE: &[Binding] = &[
    row(
        Key::Digit(3),
        Modifiers::ALT,
        ShortcutAction::ViewFileInfo,
        false,
    ), // Alt+3
    row(
        Key::Letter(b'A'),
        Modifiers::CTRL,
        ShortcutAction::ToggleAlwaysOnTop,
        false,
    ), // Ctrl+A
    row(
        Key::Letter(b'B'),
        Modifiers::NONE,
        ShortcutAction::NextTrack,
        false,
    ), // B
    row(
        Key::Letter(b'C'),
        Modifiers::NONE,
        ShortcutAction::PauseUnpause,
        false,
    ), // C
    row(
        Key::Letter(b'D'),
        Modifiers::CTRL,
        ShortcutAction::ToggleDoubleSize,
        false,
    ), // Ctrl+D
    row(
        Key::Letter(b'E'),
        Modifiers::CTRL,
        ShortcutAction::ToggleEasyMove,
        false,
    ), // Ctrl+E
    row(
        Key::Letter(b'E'),
        Modifiers::ALT,
        ShortcutAction::TogglePlaylistEditor,
        false,
    ), // Alt+E
    row(
        Key::Letter(b'F'),
        Modifiers::ALT,
        ShortcutAction::OpenMainMenu,
        true,
    ), // Alt+F
    row(
        Key::Letter(b'G'),
        Modifiers::ALT,
        ShortcutAction::ToggleEqualizer,
        false,
    ), // Alt+G
    row(
        Key::Letter(b'I'),
        Modifiers::ALT,
        ShortcutAction::AddBookmark,
        false,
    ), // Alt+I
    row(
        Key::Letter(b'I'),
        Modifiers::CTRL.union(Modifiers::ALT),
        ShortcutAction::EditBookmarks,
        false,
    ), // Ctrl+Alt+I
    row(
        Key::Letter(b'J'),
        Modifiers::NONE,
        ShortcutAction::JumpToFile,
        false,
    ), // J
    row(
        Key::Letter(b'J'),
        Modifiers::CTRL,
        ShortcutAction::JumpToTime,
        false,
    ), // Ctrl+J
    row(
        Key::Letter(b'K'),
        Modifiers::CTRL,
        ShortcutAction::SelectVisPlugin,
        false,
    ), // Ctrl+K
    row(
        Key::Letter(b'K'),
        Modifiers::ALT,
        ShortcutAction::ConfigureVisPlugin,
        false,
    ), // Alt+K
    row(
        Key::Letter(b'K'),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        ShortcutAction::ToggleVisPlugin,
        false,
    ), // Ctrl+Shift+K
    row(
        Key::Letter(b'L'),
        Modifiers::NONE,
        ShortcutAction::OpenFiles,
        false,
    ), // L
    row(
        Key::Letter(b'L'),
        Modifiers::CTRL,
        ShortcutAction::OpenLocation,
        false,
    ), // Ctrl+L
    row(
        Key::Letter(b'L'),
        Modifiers::SHIFT,
        ShortcutAction::OpenDirectory,
        false,
    ), // Shift+L
    row(
        Key::Letter(b'M'),
        Modifiers::ALT,
        ShortcutAction::Minimize,
        true,
    ), // Alt+M
    row(
        Key::Letter(b'N'),
        Modifiers::CTRL.union(Modifiers::ALT),
        ShortcutAction::NewInstance,
        true,
    ), // Ctrl+Alt+N
    row(
        Key::Letter(b'O'),
        Modifiers::ALT,
        ShortcutAction::VisOptions,
        false,
    ), // Alt+O
    row(
        Key::Letter(b'P'),
        Modifiers::CTRL,
        ShortcutAction::Preferences,
        false,
    ), // Ctrl+P
    row(
        Key::Letter(b'R'),
        Modifiers::NONE,
        ShortcutAction::ToggleRepeat,
        false,
    ), // R
    row(
        Key::Letter(b'S'),
        Modifiers::NONE,
        ShortcutAction::ToggleShuffle,
        false,
    ), // S
    row(
        Key::Letter(b'S'),
        Modifiers::ALT,
        ShortcutAction::SkinBrowser,
        false,
    ), // Alt+S
    row(
        Key::Letter(b'T'),
        Modifiers::CTRL,
        ShortcutAction::ToggleTimeRemaining,
        false,
    ), // Ctrl+T
    row(
        Key::Letter(b'T'),
        Modifiers::ALT,
        ShortcutAction::ToggleMinibrowser,
        false,
    ), // Alt+T
    row(
        Key::Letter(b'V'),
        Modifiers::NONE,
        ShortcutAction::Stop,
        false,
    ), // V
    row(
        Key::Letter(b'V'),
        Modifiers::CTRL,
        ShortcutAction::StopAfterCurrent,
        false,
    ), // Ctrl+V
    row(
        Key::Letter(b'V'),
        Modifiers::ALT,
        ShortcutAction::ToggleVideoWindow,
        false,
    ), // Alt+V
    row(
        Key::Letter(b'V'),
        Modifiers::SHIFT,
        ShortcutAction::StopWithFadeout,
        false,
    ), // Shift+V
    row(
        Key::NumDecimal,
        Modifiers::NONE,
        ShortcutAction::JumpToFile,
        false,
    ), // Num.
    row(Key::Down, Modifiers::NONE, ShortcutAction::VolumeDown, true), // Down
    row(Key::F(1), Modifiers::NONE, ShortcutAction::About, false),     // F1
    row(
        Key::F(3),
        Modifiers::NONE,
        ShortcutAction::PlaylistSearch,
        true,
    ), // F3
    row(
        Key::F(4),
        Modifiers::CTRL,
        ShortcutAction::CloseFocusedWindow,
        true,
    ), // Ctrl+F4
    row(
        Key::F(5),
        Modifiers::NONE,
        ShortcutAction::RefreshSkin,
        true,
    ), // F5
    row(
        Key::Insert,
        Modifiers::NONE,
        ShortcutAction::OpenDirectory,
        false,
    ), // Insert
    row(Key::Left, Modifiers::NONE, ShortcutAction::Rewind5s, false),  // Left
    row(
        Key::Numpad(0),
        Modifiers::NONE,
        ShortcutAction::OpenFiles,
        false,
    ), // Num0
    row(
        Key::Numpad(0),
        Modifiers::CTRL,
        ShortcutAction::OpenLocation,
        false,
    ), // Ctrl+Num0
    row(
        Key::Numpad(1),
        Modifiers::NONE,
        ShortcutAction::TenTracksBack,
        false,
    ), // Num1
    row(
        Key::Numpad(2),
        Modifiers::NONE,
        ShortcutAction::VolumeDown,
        true,
    ), // Num2
    row(
        Key::Numpad(3),
        Modifiers::NONE,
        ShortcutAction::TenTracksForward,
        false,
    ), // Num3
    row(
        Key::Numpad(4),
        Modifiers::NONE,
        ShortcutAction::PreviousTrack,
        false,
    ), // Num4
    row(Key::Numpad(5), Modifiers::NONE, ShortcutAction::Play, false), // Num5
    row(
        Key::Numpad(6),
        Modifiers::NONE,
        ShortcutAction::NextTrack,
        false,
    ), // Num6
    row(
        Key::Numpad(7),
        Modifiers::NONE,
        ShortcutAction::Rewind5s,
        false,
    ), // Num7
    row(
        Key::Numpad(8),
        Modifiers::NONE,
        ShortcutAction::VolumeUp,
        true,
    ), // Num8
    row(
        Key::Numpad(9),
        Modifiers::NONE,
        ShortcutAction::Forward5s,
        false,
    ), // Num9
    row(
        Key::Enter,
        Modifiers::ALT,
        ShortcutAction::ToggleVideoFullscreen,
        true,
    ), // Alt+Enter
    row(
        Key::Right,
        Modifiers::NONE,
        ShortcutAction::Forward5s,
        false,
    ), // Right
    row(Key::Tab, Modifiers::CTRL, ShortcutAction::NextWindow, true),  // Ctrl+Tab
    row(Key::Up, Modifiers::NONE, ShortcutAction::VolumeUp, true),     // Up
    row(
        Key::Letter(b'W'),
        Modifiers::CTRL,
        ShortcutAction::ToggleWindowShade,
        false,
    ), // Ctrl+W (S03 §1: observed as a command)
    row(
        Key::Letter(b'W'),
        Modifiers::ALT,
        ShortcutAction::ShowMainWindow,
        false,
    ), // Alt+W
    row(
        Key::Letter(b'X'),
        Modifiers::NONE,
        ShortcutAction::Play,
        false,
    ), // X
    row(
        Key::Letter(b'X'),
        Modifiers::CTRL,
        ShortcutAction::OpenLocation,
        false,
    ), // Ctrl+X
    row(
        Key::Letter(b'X'),
        Modifiers::SHIFT,
        ShortcutAction::OpenFiles,
        false,
    ), // Shift+X
    row(
        Key::Letter(b'Z'),
        Modifiers::NONE,
        ShortcutAction::PreviousTrack,
        false,
    ), // Z
    row(
        Key::Letter(b'Z'),
        Modifiers::CTRL,
        ShortcutAction::StartOfList,
        false,
    ), // Ctrl+Z
];

/// The global table, S03 §4: 29 rows in the spec's order.
///
/// **Provisional** (S03 §4, §6.1): whether it is active together with a window
/// table is unconfirmed; [`lookup`] consults it after the window's table.
pub const GLOBAL_TABLE: &[Binding] = &[
    row(
        Key::Letter(b'A'),
        Modifiers::CTRL,
        ShortcutAction::ToggleAlwaysOnTop,
        false,
    ), // Ctrl+A
    row(
        Key::Letter(b'D'),
        Modifiers::CTRL,
        ShortcutAction::ToggleDoubleSize,
        false,
    ), // Ctrl+D
    row(
        Key::Letter(b'E'),
        Modifiers::CTRL,
        ShortcutAction::ToggleEasyMove,
        false,
    ), // Ctrl+E
    row(
        Key::Letter(b'E'),
        Modifiers::ALT,
        ShortcutAction::TogglePlaylistEditor,
        false,
    ), // Alt+E
    row(
        Key::Letter(b'F'),
        Modifiers::ALT,
        ShortcutAction::OpenMainMenu,
        true,
    ), // Alt+F
    row(
        Key::Letter(b'G'),
        Modifiers::ALT,
        ShortcutAction::ToggleEqualizer,
        false,
    ), // Alt+G
    row(
        Key::Letter(b'K'),
        Modifiers::CTRL,
        ShortcutAction::SelectVisPlugin,
        false,
    ), // Ctrl+K
    row(
        Key::Letter(b'K'),
        Modifiers::ALT,
        ShortcutAction::ConfigureVisPlugin,
        false,
    ), // Alt+K
    row(
        Key::Letter(b'K'),
        Modifiers::CTRL.union(Modifiers::SHIFT),
        ShortcutAction::ToggleVisPlugin,
        false,
    ), // Ctrl+Shift+K
    row(
        Key::Letter(b'P'),
        Modifiers::CTRL,
        ShortcutAction::Preferences,
        false,
    ), // Ctrl+P
    row(
        Key::Letter(b'S'),
        Modifiers::ALT,
        ShortcutAction::SkinBrowser,
        false,
    ), // Alt+S
    row(
        Key::Letter(b'T'),
        Modifiers::CTRL,
        ShortcutAction::ToggleTimeRemaining,
        false,
    ), // Ctrl+T
    row(
        Key::Letter(b'T'),
        Modifiers::ALT,
        ShortcutAction::ToggleMinibrowser,
        false,
    ), // Alt+T
    row(
        Key::Letter(b'V'),
        Modifiers::ALT,
        ShortcutAction::ToggleVideoWindow,
        false,
    ), // Alt+V
    row(
        Key::NumDecimal,
        Modifiers::NONE,
        ShortcutAction::JumpToFile,
        false,
    ), // Num.
    row(Key::F(1), Modifiers::NONE, ShortcutAction::About, false), // F1
    row(
        Key::F(3),
        Modifiers::NONE,
        ShortcutAction::PlaylistSearch,
        true,
    ), // F3
    row(
        Key::F(4),
        Modifiers::CTRL,
        ShortcutAction::CloseFocusedWindow,
        true,
    ), // Ctrl+F4
    row(
        Key::F(5),
        Modifiers::NONE,
        ShortcutAction::RefreshSkin,
        true,
    ), // F5
    row(
        Key::Enter,
        Modifiers::ALT,
        ShortcutAction::ToggleVideoFullscreen,
        true,
    ), // Alt+Enter
    row(Key::Tab, Modifiers::CTRL, ShortcutAction::NextWindow, true), // Ctrl+Tab
    row(
        Key::Letter(b'W'),
        Modifiers::CTRL,
        ShortcutAction::ToggleWindowShade,
        false,
    ), // Ctrl+W (S03 §1: observed as a command)
    row(
        Key::Letter(b'W'),
        Modifiers::ALT,
        ShortcutAction::ShowMainWindow,
        false,
    ), // Alt+W
    row(
        Key::Letter(b'L'),
        Modifiers::CTRL,
        ShortcutAction::MinibrowserCtrlL,
        true,
    ), // Ctrl+L
    row(
        Key::Letter(b'O'),
        Modifiers::CTRL,
        ShortcutAction::MinibrowserCtrlO,
        true,
    ), // Ctrl+O
    row(
        Key::Letter(b'R'),
        Modifiers::CTRL,
        ShortcutAction::MinibrowserCtrlR,
        true,
    ), // Ctrl+R
    row(
        Key::Letter(b'R'),
        Modifiers::CTRL.union(Modifiers::ALT),
        ShortcutAction::MinibrowserCtrlAltR,
        true,
    ), // Ctrl+Alt+R
    row(
        Key::Left,
        Modifiers::ALT,
        ShortcutAction::MinibrowserAltLeft,
        true,
    ), // Alt+Left
    row(
        Key::Right,
        Modifiers::ALT,
        ShortcutAction::MinibrowserAltRight,
        true,
    ), // Alt+Right
];

fn find(table: &[Binding], key: Key, modifiers: Modifiers) -> Option<&Binding> {
    let key = match key {
        Key::Letter(c) => Key::Letter(c.to_ascii_uppercase()),
        other => other,
    };
    table
        .iter()
        .find(|b| b.key == key && b.modifiers == modifiers)
}

/// The binding for a key press in `window`, or `None`.
///
/// **Provisional** (S03 §4, §6.1): the window's table is checked first, then
/// the global table. On a conflict the window's table wins (`Ctrl+L` is "Open
/// location" in the main table and a minibrowser command globally).
pub fn lookup_binding(
    window: ShortcutWindow,
    key: Key,
    modifiers: Modifiers,
) -> Option<&'static Binding> {
    let own: &'static [Binding] = match window {
        ShortcutWindow::Main => MAIN_TABLE,
    };
    find(own, key, modifiers).or_else(|| find(GLOBAL_TABLE, key, modifiers))
}

/// The action for a key press in `window`, or `None` (see [`lookup_binding`]).
pub fn lookup(window: ShortcutWindow, key: Key, modifiers: Modifiers) -> Option<ShortcutAction> {
    lookup_binding(window, key, modifiers).map(|b| b.action)
}

/// What dispatching a shortcut produced.
#[derive(Debug, Clone, PartialEq)]
pub enum ShortcutOutcome {
    /// The action maps onto existing commands. The machine has already applied
    /// its S02 prediction; the caller forwards them to the core.
    Commands(Vec<Command>),
    /// The effect stayed inside the machine (for example the elapsed/remaining
    /// time toggle).
    Handled,
    /// The action has no [`Command`] yet; the app layer handles it later.
    Unhandled(ShortcutAction),
}

impl MainWindow {
    /// Dispatches a key press in the main window.
    ///
    /// Returns `None` when the key is unbound. A repeated press is dispatched
    /// again (see the module docs).
    pub fn key_down(
        &mut self,
        key: Key,
        modifiers: Modifiers,
        now_ms: u64,
    ) -> Option<ShortcutOutcome> {
        let action = lookup(ShortcutWindow::Main, key, modifiers)?;
        Some(self.dispatch_shortcut(action, now_ms))
    }

    /// Dispatches one action (S03 §1) against the machine.
    ///
    /// The match is exhaustive on purpose: a new [`ShortcutAction`] must be
    /// given a verdict here (a `Command`, `Handled` or `Unhandled` with a
    /// reason) instead of falling into a catch-all.
    pub fn dispatch_shortcut(&mut self, action: ShortcutAction, now_ms: u64) -> ShortcutOutcome {
        use ShortcutAction as A;
        let commands = match action {
            A::Play => vec![Command::Play],
            A::PauseUnpause => vec![Command::TogglePause],
            A::Stop => vec![Command::Stop],
            A::StopWithFadeout => vec![Command::StopWithFadeout],
            A::NextTrack => vec![Command::Next],
            A::PreviousTrack => vec![Command::Prev],
            A::ToggleShuffle => vec![Command::ToggleShuffle],
            A::ToggleRepeat => vec![Command::ToggleRepeat],
            A::Rewind5s => vec![Command::SeekRelative {
                delta_ms: -SEEK_STEP_MS,
            }],
            A::Forward5s => vec![Command::SeekRelative {
                delta_ms: SEEK_STEP_MS,
            }],
            A::ToggleDoubleSize => vec![Command::ToggleDoubleSize],
            A::ToggleAlwaysOnTop => vec![Command::ToggleAlwaysOnTop],
            A::ToggleEqualizer => vec![Command::ToggleWindow(WindowId::Equalizer)],
            A::TogglePlaylistEditor => vec![Command::ToggleWindow(WindowId::Playlist)],
            // Provisional key (S03 §1, §6.2); the action was exercised.
            A::ToggleWindowShade => vec![Command::ToggleWindowShade(WindowId::Main)],
            A::Preferences => vec![Command::OpenPrefs],
            A::VolumeUp => vec![self.volume_command(true)],
            A::VolumeDown => vec![self.volume_command(false)],
            A::ToggleTimeRemaining => {
                self.toggle_time_remaining(now_ms);
                return ShortcutOutcome::Handled;
            }

            // ----- RFC 0005: actions that reuse an existing command -----
            // Start/end of list play (owner decision O1); `PlayLast` is the
            // engine's job for the mouse-only "end of list" action (S03 §5).
            A::StartOfList => vec![Command::PlayIndex(0)],
            A::ToggleVideoWindow => vec![Command::ToggleWindow(WindowId::Video)],

            // ----- RFC 0005: new engine commands -----
            A::StopAfterCurrent => vec![Command::ToggleStopAfterCurrent],
            A::TenTracksForward => vec![Command::SkipBy {
                delta: TEN_TRACK_STEP,
            }],
            A::TenTracksBack => vec![Command::SkipBy {
                delta: -TEN_TRACK_STEP,
            }],

            // ----- RFC 0005: new app commands (dialogs) -----
            A::OpenFiles => vec![Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            }],
            A::OpenLocation => vec![Command::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            }],
            A::OpenDirectory => vec![Command::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            }],
            A::JumpToFile => vec![Command::OpenJump {
                target: JumpTarget::File,
            }],
            A::JumpToTime => vec![Command::OpenJump {
                target: JumpTarget::Time,
            }],
            A::ViewFileInfo => vec![Command::OpenFileInfo],
            A::SelectVisPlugin => vec![Command::OpenPrefsPage(PrefsPage::VisPlugin)],
            A::VisOptions => vec![Command::OpenPrefsPage(PrefsPage::VisOptions)],
            A::About => vec![Command::OpenAbout],
            A::SkinBrowser => vec![Command::OpenSkinBrowser],
            A::RefreshSkin => vec![Command::RefreshSkin],
            A::EditBookmarks => vec![Command::OpenBookmarkEditor],
            A::AddBookmark => vec![Command::AddBookmark],

            // ----- RFC 0005: new app commands (window management) -----
            A::ToggleEasyMove => vec![Command::ToggleEasyMove],
            A::Minimize => vec![Command::Minimize],
            A::NextWindow => vec![Command::FocusNextWindow],
            A::ShowMainWindow => vec![Command::ShowMainWindow],
            A::CloseFocusedWindow => vec![Command::CloseFocusedWindow],
            A::OpenMainMenu => vec![Command::OpenMainMenu],

            // ----- RFC 0005: app -> host bridge (handling deferred to M3) -----
            A::ConfigureVisPlugin => vec![Command::ConfigureVisPlugin],
            A::ToggleVisPlugin => vec![Command::ToggleVisPlugin],

            // Deferred (RFC 0005 §11): the minibrowser, the video-fullscreen
            // mode and the playlist search have no window, menu or plug-in
            // bridge yet, so there is nothing to send. The host logs and drops
            // these ids.
            A::ToggleMinibrowser
            | A::MinibrowserAltRight
            | A::MinibrowserAltLeft
            | A::MinibrowserCtrlR
            | A::MinibrowserCtrlL
            | A::MinibrowserCtrlO
            | A::MinibrowserCtrlAltR
            | A::PlaylistSearch
            | A::ToggleVideoFullscreen
            // App-local (RFC 0005 §10): the effect is a new process, entirely
            // outside the player, so there is deliberately no variant. The
            // platform layer consumes this `Unhandled` action.
            | A::NewInstance => return ShortcutOutcome::Unhandled(action),
        };
        for command in &commands {
            self.handle_command(command, now_ms);
        }
        ShortcutOutcome::Commands(commands)
    }

    /// `SetVolume` one [`VOLUME_STEP`] above or below the current volume,
    /// clamped to `0..=255`.
    fn volume_command(&self, up: bool) -> Command {
        let current = core_volume(self.state().volume);
        let next = if up {
            current.saturating_add(VOLUME_STEP)
        } else {
            current.saturating_sub(VOLUME_STEP)
        };
        Command::SetVolume(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::state::TransportState;

    const M: ShortcutWindow = ShortcutWindow::Main;

    fn k(c: char) -> Key {
        Key::letter(c).unwrap()
    }

    fn press(machine: &mut MainWindow, key: Key, mods: Modifiers) -> Option<ShortcutOutcome> {
        machine.key_down(key, mods, 0)
    }

    fn cmds(machine: &mut MainWindow, key: Key, mods: Modifiers) -> Vec<Command> {
        match press(machine, key, mods) {
            Some(ShortcutOutcome::Commands(c)) => c,
            other => panic!("expected commands, got {other:?}"),
        }
    }

    /// The S03 §1 key rows, written out independently of the table:
    /// (key, modifiers, command id).
    fn s03_main_rows() -> Vec<(Key, Modifiers, u32)> {
        let n = Modifiers::NONE;
        let c = Modifiers::CTRL;
        let a = Modifiers::ALT;
        let s = Modifiers::SHIFT;
        vec![
            (Key::Digit(3), a, 40188),
            (k('A'), c, 40019),
            (k('B'), n, 40048),
            (k('C'), n, 40046),
            (k('D'), c, 40165),
            (k('E'), c, 40186),
            (k('E'), a, 40040),
            (k('F'), a, 40043),
            (k('G'), a, 40036),
            (k('I'), a, 40321),
            (k('I'), c | a, 40320),
            (k('J'), n, 40194),
            (k('J'), c, 40193),
            (k('K'), c, 40191),
            (k('K'), a, 40221),
            (k('K'), c | s, 40192),
            (k('L'), n, 40029),
            (k('L'), c, 40185),
            (k('L'), s, 40187),
            (k('M'), a, 40334),
            (k('N'), c | a, 40305),
            (k('O'), a, 40317),
            (k('P'), c, 40012),
            (k('R'), n, 40022),
            (k('S'), n, 40023),
            (k('S'), a, 40219),
            (k('T'), c, 40242),
            (k('T'), a, 40298),
            (k('V'), n, 40047),
            (k('V'), c, 40157),
            (k('V'), a, 40328),
            (k('V'), s, 40147),
            (Key::NumDecimal, n, 40194),
            (Key::Down, n, 40059),
            (Key::F(1), n, 40041),
            (Key::F(3), n, 40318),
            (Key::F(4), c, 40224),
            (Key::F(5), n, 40291),
            (Key::Insert, n, 40187),
            (Key::Left, n, 40061),
            (Key::Numpad(0), n, 40029),
            (Key::Numpad(0), c, 40185),
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
            (Key::Right, n, 40060),
            (Key::Tab, c, 40063),
            (Key::Up, n, 40058),
            (k('W'), c, 40064),
            (k('W'), a, 40258),
            (k('X'), n, 40045),
            (k('X'), c, 40185),
            (k('X'), s, 40029),
            (k('Z'), n, 40044),
            (k('Z'), c, 40154),
        ]
    }

    // ----- table coverage -----

    #[test]
    fn main_table_has_exactly_the_s03_row_count() {
        assert_eq!(s03_main_rows().len(), 62);
        assert_eq!(MAIN_TABLE.len(), 62);
        assert_eq!(GLOBAL_TABLE.len(), 29);
    }

    #[test]
    fn every_s03_main_row_resolves() {
        for (key, mods, id) in s03_main_rows() {
            let action = lookup(M, key, mods)
                .unwrap_or_else(|| panic!("no binding for {key:?} {mods:?} (id {id})"));
            assert_eq!(action.command_id(), id, "{key:?} {mods:?}");
        }
    }

    #[test]
    fn main_table_matches_the_independent_row_list_exactly() {
        let rows = s03_main_rows();
        assert_eq!(rows.len(), MAIN_TABLE.len());
        for (binding, (key, mods, id)) in MAIN_TABLE.iter().zip(rows) {
            assert_eq!(binding.key, key);
            assert_eq!(binding.modifiers, mods);
            assert_eq!(binding.action.command_id(), id);
        }
    }

    #[test]
    fn tables_have_no_duplicate_key_rows() {
        for table in [MAIN_TABLE, GLOBAL_TABLE] {
            for (i, a) in table.iter().enumerate() {
                for b in &table[i + 1..] {
                    assert!(
                        !(a.key == b.key && a.modifiers == b.modifiers),
                        "{:?}",
                        a.key
                    );
                }
            }
        }
    }

    #[test]
    fn provisional_rows_are_flagged() {
        let provisional: Vec<u32> = MAIN_TABLE
            .iter()
            .filter(|b| b.provisional)
            .map(|b| b.action.command_id())
            .collect();
        // Alt+F, Alt+M, Ctrl+Alt+N, Down, F3, Ctrl+F4, F5, Num2, Num8,
        // Alt+Enter, Ctrl+Tab, Up. Ctrl+W (40064) is observed, not provisional.
        assert_eq!(provisional.len(), 12);
        assert!(!provisional.contains(&40064));
        for id in [
            40043, 40334, 40305, 40059, 40318, 40224, 40291, 40058, 40337, 40063,
        ] {
            assert!(provisional.contains(&id), "{id}");
        }
    }

    // ----- no stray bindings -----

    #[test]
    fn unbound_keys_map_to_nothing() {
        assert_eq!(lookup(M, k('Q'), Modifiers::NONE), None);
        assert_eq!(lookup(M, k('Q'), Modifiers::CTRL), None);
        assert_eq!(lookup(M, Key::Digit(1), Modifiers::NONE), None);
        assert_eq!(lookup(M, Key::Grave, Modifiers::NONE), None);
        assert_eq!(lookup(M, Key::Delete, Modifiers::NONE), None);
        assert_eq!(lookup(M, Key::Home, Modifiers::NONE), None);
        assert_eq!(lookup(M, Key::Space, Modifiers::NONE), None);
        assert_eq!(lookup(M, Key::NumAdd, Modifiers::CTRL), None);
        assert_eq!(lookup(M, Key::Left, Modifiers::SHIFT), None);
        assert_eq!(lookup(M, k('B'), Modifiers::CTRL), None);
        let mut machine = MainWindow::new();
        assert_eq!(press(&mut machine, k('Q'), Modifiers::NONE), None);
    }

    #[test]
    fn letters_are_case_insensitive_in_the_key_type() {
        assert_eq!(Key::letter('l'), Key::letter('L'));
        assert_eq!(Key::letter('1'), None);
        assert_eq!(
            lookup(M, Key::Letter(b'b'), Modifiers::NONE),
            lookup(M, k('B'), Modifiers::NONE)
        );
    }

    // ----- modifiers -----

    #[test]
    fn modifier_variants_resolve_to_different_actions() {
        let l = |m| lookup(M, k('L'), m).map(ShortcutAction::command_id);
        assert_eq!(l(Modifiers::NONE), Some(40029));
        assert_eq!(l(Modifiers::SHIFT), Some(40187));
        assert_eq!(l(Modifiers::CTRL), Some(40185));
        assert_eq!(l(Modifiers::ALT), None);
        assert_eq!(l(Modifiers::CTRL | Modifiers::SHIFT), None);
        let kk = |m| lookup(M, k('K'), m).map(ShortcutAction::command_id);
        assert_eq!(kk(Modifiers::CTRL), Some(40191));
        assert_eq!(kk(Modifiers::CTRL | Modifiers::SHIFT), Some(40192));
        assert_eq!(kk(Modifiers::ALT), Some(40221));
        assert_eq!(kk(Modifiers::NONE), None);
        assert!(Modifiers::new(true, false, true).shift());
        assert!(!Modifiers::new(true, false, true).alt());
    }

    // ----- table combination (provisional) -----

    #[test]
    fn provisional_ctrl_l_resolves_to_the_main_table_meaning() {
        // provisional: S03 §4/§6.1 do not say which table wins.
        assert_eq!(
            lookup(M, k('L'), Modifiers::CTRL),
            Some(ShortcutAction::OpenLocation)
        );
        assert_eq!(
            find(GLOBAL_TABLE, k('L'), Modifiers::CTRL).map(|b| b.action.command_id()),
            Some(40302)
        );
    }

    #[test]
    fn provisional_global_only_keys_fall_through_to_the_global_table() {
        // Ctrl+O is not in the main table; the global table binds it.
        assert_eq!(find(MAIN_TABLE, k('O'), Modifiers::CTRL), None);
        assert_eq!(
            lookup(M, k('O'), Modifiers::CTRL).map(ShortcutAction::command_id),
            Some(40303)
        );
        assert_eq!(
            lookup(M, Key::Left, Modifiers::ALT).map(ShortcutAction::command_id),
            Some(40300)
        );
    }

    // ----- each mapped action -----

    #[test]
    fn transport_keys_emit_the_right_commands() {
        let mut m = MainWindow::new();
        let n = Modifiers::NONE;
        assert_eq!(cmds(&mut m, k('X'), n), vec![Command::Play]);
        assert_eq!(cmds(&mut m, Key::Numpad(5), n), vec![Command::Play]);
        assert_eq!(cmds(&mut m, k('C'), n), vec![Command::TogglePause]);
        assert_eq!(cmds(&mut m, k('V'), n), vec![Command::Stop]);
        assert_eq!(cmds(&mut m, k('B'), n), vec![Command::Next]);
        assert_eq!(cmds(&mut m, Key::Numpad(6), n), vec![Command::Next]);
        assert_eq!(cmds(&mut m, k('Z'), n), vec![Command::Prev]);
        assert_eq!(cmds(&mut m, Key::Numpad(4), n), vec![Command::Prev]);
        assert_eq!(
            cmds(&mut m, k('V'), Modifiers::SHIFT),
            vec![Command::StopWithFadeout]
        );
    }

    #[test]
    fn transport_keys_drive_the_machine_state() {
        let mut m = MainWindow::new();
        cmds(&mut m, k('X'), Modifiers::NONE);
        assert_eq!(m.state().transport, TransportState::Playing);
        cmds(&mut m, k('C'), Modifiers::NONE);
        assert_eq!(m.state().transport, TransportState::Paused);
        cmds(&mut m, k('V'), Modifiers::NONE);
        assert_eq!(m.state().transport, TransportState::Stopped);
    }

    #[test]
    fn seek_keys_emit_relative_seeks() {
        let mut m = MainWindow::new();
        let n = Modifiers::NONE;
        let back = vec![Command::SeekRelative { delta_ms: -5000 }];
        let fwd = vec![Command::SeekRelative { delta_ms: 5000 }];
        assert_eq!(cmds(&mut m, Key::Left, n), back);
        assert_eq!(cmds(&mut m, Key::Numpad(7), n), back);
        assert_eq!(cmds(&mut m, Key::Right, n), fwd);
        assert_eq!(cmds(&mut m, Key::Numpad(9), n), fwd);
    }

    #[test]
    fn toggle_keys_emit_their_commands() {
        let mut m = MainWindow::new();
        let n = Modifiers::NONE;
        let c = Modifiers::CTRL;
        let a = Modifiers::ALT;
        assert_eq!(cmds(&mut m, k('S'), n), vec![Command::ToggleShuffle]);
        assert!(m.state().shuffle);
        assert_eq!(cmds(&mut m, k('R'), n), vec![Command::ToggleRepeat]);
        assert!(m.state().repeat);
        assert_eq!(cmds(&mut m, k('D'), c), vec![Command::ToggleDoubleSize]);
        assert!(m.state().double_size);
        assert_eq!(cmds(&mut m, k('A'), c), vec![Command::ToggleAlwaysOnTop]);
        assert_eq!(
            cmds(&mut m, k('W'), c),
            vec![Command::ToggleWindowShade(WindowId::Main)]
        );
        assert!(m.state().windowshade);
        assert_eq!(cmds(&mut m, k('P'), c), vec![Command::OpenPrefs]);
        assert_eq!(
            cmds(&mut m, k('G'), a),
            vec![Command::ToggleWindow(WindowId::Equalizer)]
        );
        assert!(m.state().eq_open);
        assert_eq!(
            cmds(&mut m, k('E'), a),
            vec![Command::ToggleWindow(WindowId::Playlist)]
        );
        assert!(m.state().playlist_open);
    }

    // ----- Handled -----

    #[test]
    fn ctrl_t_toggles_elapsed_remaining_locally() {
        let mut m = MainWindow::new();
        assert!(!m.state().time_remaining);
        assert_eq!(
            press(&mut m, k('T'), Modifiers::CTRL),
            Some(ShortcutOutcome::Handled)
        );
        assert!(m.state().time_remaining);
        assert_eq!(
            press(&mut m, k('T'), Modifiers::CTRL),
            Some(ShortcutOutcome::Handled)
        );
        assert!(!m.state().time_remaining);
    }

    // ----- Unhandled (RFC 0005 §10/§11) -----

    /// The actions with no `Command`: the deferred ones and the app-local one.
    fn deferred_actions() -> Vec<ShortcutAction> {
        vec![
            // Minibrowser (RFC 0005 §11.1): no window, no owning spec.
            ShortcutAction::ToggleMinibrowser,
            ShortcutAction::MinibrowserAltRight,
            ShortcutAction::MinibrowserAltLeft,
            ShortcutAction::MinibrowserCtrlR,
            ShortcutAction::MinibrowserCtrlL,
            ShortcutAction::MinibrowserCtrlO,
            ShortcutAction::MinibrowserCtrlAltR,
            // Video fullscreen (§11.2) and playlist search (§11.3).
            ShortcutAction::ToggleVideoFullscreen,
            ShortcutAction::PlaylistSearch,
            // App-local (§10): a new process, outside the player.
            ShortcutAction::NewInstance,
        ]
    }

    /// Every distinct `ShortcutAction`, written out independently of the table.
    fn all_actions() -> Vec<ShortcutAction> {
        vec![
            ShortcutAction::Preferences,
            ShortcutAction::ToggleAlwaysOnTop,
            ShortcutAction::ToggleRepeat,
            ShortcutAction::ToggleShuffle,
            ShortcutAction::OpenFiles,
            ShortcutAction::ToggleEqualizer,
            ShortcutAction::TogglePlaylistEditor,
            ShortcutAction::About,
            ShortcutAction::OpenMainMenu,
            ShortcutAction::PreviousTrack,
            ShortcutAction::Play,
            ShortcutAction::PauseUnpause,
            ShortcutAction::Stop,
            ShortcutAction::NextTrack,
            ShortcutAction::VolumeUp,
            ShortcutAction::VolumeDown,
            ShortcutAction::Forward5s,
            ShortcutAction::Rewind5s,
            ShortcutAction::NextWindow,
            ShortcutAction::ToggleWindowShade,
            ShortcutAction::StopWithFadeout,
            ShortcutAction::StartOfList,
            ShortcutAction::StopAfterCurrent,
            ShortcutAction::ToggleDoubleSize,
            ShortcutAction::OpenLocation,
            ShortcutAction::ToggleEasyMove,
            ShortcutAction::OpenDirectory,
            ShortcutAction::ViewFileInfo,
            ShortcutAction::SelectVisPlugin,
            ShortcutAction::ToggleVisPlugin,
            ShortcutAction::JumpToTime,
            ShortcutAction::JumpToFile,
            ShortcutAction::TenTracksForward,
            ShortcutAction::TenTracksBack,
            ShortcutAction::SkinBrowser,
            ShortcutAction::ConfigureVisPlugin,
            ShortcutAction::CloseFocusedWindow,
            ShortcutAction::ToggleTimeRemaining,
            ShortcutAction::ShowMainWindow,
            ShortcutAction::RefreshSkin,
            ShortcutAction::ToggleMinibrowser,
            ShortcutAction::MinibrowserAltRight,
            ShortcutAction::MinibrowserAltLeft,
            ShortcutAction::MinibrowserCtrlR,
            ShortcutAction::MinibrowserCtrlL,
            ShortcutAction::MinibrowserCtrlO,
            ShortcutAction::NewInstance,
            ShortcutAction::MinibrowserCtrlAltR,
            ShortcutAction::VisOptions,
            ShortcutAction::PlaylistSearch,
            ShortcutAction::EditBookmarks,
            ShortcutAction::AddBookmark,
            ShortcutAction::ToggleVideoWindow,
            ShortcutAction::Minimize,
            ShortcutAction::ToggleVideoFullscreen,
        ]
    }

    #[test]
    fn there_are_exactly_the_s03_action_count() {
        let actions = all_actions();
        assert_eq!(actions.len(), 55);
        let distinct: std::collections::BTreeSet<_> =
            actions.iter().map(|a| a.command_id()).collect();
        assert_eq!(distinct.len(), 55, "the ids are distinct");
        let deferred: std::collections::BTreeSet<_> =
            deferred_actions().iter().map(|a| a.command_id()).collect();
        assert_eq!(deferred.len(), 10);
    }

    #[test]
    fn unhandled_is_exactly_the_deferred_set_and_new_instance() {
        let deferred = deferred_actions();
        for action in all_actions() {
            let mut m = MainWindow::new();
            let outcome = m.dispatch_shortcut(action, 0);
            let is_unhandled = matches!(outcome, ShortcutOutcome::Unhandled(_));
            assert_eq!(
                is_unhandled,
                deferred.contains(&action),
                "{action:?} -> {outcome:?}"
            );
            if is_unhandled {
                assert_eq!(outcome, ShortcutOutcome::Unhandled(action));
            }
        }
    }

    #[test]
    fn the_deferred_and_app_local_actions_leave_the_machine_untouched() {
        for action in deferred_actions() {
            let mut m = MainWindow::new();
            let before = m.state().clone();
            assert_eq!(
                m.dispatch_shortcut(action, 0),
                ShortcutOutcome::Unhandled(action),
                "{action:?}"
            );
            assert_eq!(m.state(), &before, "{action:?}");
        }
    }

    #[test]
    fn every_other_action_produces_a_command_or_is_handled() {
        let deferred = deferred_actions();
        for action in all_actions() {
            if deferred.contains(&action) {
                continue;
            }
            let mut m = MainWindow::new();
            match m.dispatch_shortcut(action, 0) {
                ShortcutOutcome::Commands(commands) => {
                    assert!(!commands.is_empty(), "{action:?}");
                }
                ShortcutOutcome::Handled => {
                    // Only the UI-local elapsed/remaining toggle (RFC 0005 §9).
                    assert_eq!(action, ShortcutAction::ToggleTimeRemaining);
                }
                other => panic!("{action:?} -> {other:?}"),
            }
        }
    }

    #[test]
    fn the_main_table_rows_that_were_unhandled_now_map_or_stay_deferred() {
        let deferred_ids: std::collections::BTreeSet<u32> =
            deferred_actions().iter().map(|a| a.command_id()).collect();
        let mut unhandled = Vec::new();
        for (key, mods, id) in s03_main_rows() {
            let mut m = MainWindow::new();
            if let Some(ShortcutOutcome::Unhandled(action)) = press(&mut m, key, mods) {
                assert_eq!(action.command_id(), id);
                assert!(deferred_ids.contains(&id), "id {id} must not be unhandled");
                unhandled.push(id);
            }
        }
        // Reachable from the main table: the minibrowser toggle, the app-local
        // new instance, the playlist search and the video fullscreen.
        let distinct: std::collections::BTreeSet<u32> = unhandled.iter().copied().collect();
        assert_eq!(distinct, [40298, 40305, 40318, 40337].into_iter().collect());
    }

    #[test]
    fn unhandled_leaves_the_machine_untouched() {
        let mut m = MainWindow::new();
        let before = m.state().clone();
        assert_eq!(
            press(&mut m, Key::F(3), Modifiers::NONE),
            Some(ShortcutOutcome::Unhandled(ShortcutAction::PlaylistSearch))
        );
        assert_eq!(m.state(), &before);
    }

    // ----- RFC 0005 dispatch -----

    #[test]
    fn the_rfc_0005_actions_emit_their_commands() {
        let cases: [(ShortcutAction, Command); 26] = [
            (ShortcutAction::ViewFileInfo, Command::OpenFileInfo),
            (ShortcutAction::ToggleEasyMove, Command::ToggleEasyMove),
            (ShortcutAction::OpenMainMenu, Command::OpenMainMenu),
            (ShortcutAction::AddBookmark, Command::AddBookmark),
            (ShortcutAction::EditBookmarks, Command::OpenBookmarkEditor),
            (
                ShortcutAction::JumpToFile,
                Command::OpenJump {
                    target: JumpTarget::File,
                },
            ),
            (
                ShortcutAction::JumpToTime,
                Command::OpenJump {
                    target: JumpTarget::Time,
                },
            ),
            (
                ShortcutAction::SelectVisPlugin,
                Command::OpenPrefsPage(PrefsPage::VisPlugin),
            ),
            (
                ShortcutAction::VisOptions,
                Command::OpenPrefsPage(PrefsPage::VisOptions),
            ),
            (
                ShortcutAction::ConfigureVisPlugin,
                Command::ConfigureVisPlugin,
            ),
            (ShortcutAction::ToggleVisPlugin, Command::ToggleVisPlugin),
            (
                ShortcutAction::OpenFiles,
                Command::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: true,
                },
            ),
            (
                ShortcutAction::OpenLocation,
                Command::OpenMedia {
                    kind: OpenMediaKind::Location,
                    play: true,
                },
            ),
            (
                ShortcutAction::OpenDirectory,
                Command::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play: true,
                },
            ),
            (ShortcutAction::Minimize, Command::Minimize),
            (ShortcutAction::SkinBrowser, Command::OpenSkinBrowser),
            (
                ShortcutAction::StopAfterCurrent,
                Command::ToggleStopAfterCurrent,
            ),
            (
                ShortcutAction::ToggleVideoWindow,
                Command::ToggleWindow(WindowId::Video),
            ),
            (ShortcutAction::About, Command::OpenAbout),
            (
                ShortcutAction::CloseFocusedWindow,
                Command::CloseFocusedWindow,
            ),
            (ShortcutAction::RefreshSkin, Command::RefreshSkin),
            (
                ShortcutAction::TenTracksBack,
                Command::SkipBy { delta: -10 },
            ),
            (
                ShortcutAction::TenTracksForward,
                Command::SkipBy { delta: 10 },
            ),
            (ShortcutAction::NextWindow, Command::FocusNextWindow),
            (ShortcutAction::ShowMainWindow, Command::ShowMainWindow),
            (ShortcutAction::StartOfList, Command::PlayIndex(0)),
        ];
        for (action, expected) in cases {
            let mut m = MainWindow::new();
            assert_eq!(
                m.dispatch_shortcut(action, 0),
                ShortcutOutcome::Commands(vec![expected]),
                "{action:?}"
            );
        }
    }

    #[test]
    fn the_rfc_0005_actions_are_reachable_from_their_s03_keys() {
        let cases: [(Key, Modifiers, Command); 18] = [
            (Key::Digit(3), Modifiers::ALT, Command::OpenFileInfo),
            (k('E'), Modifiers::CTRL, Command::ToggleEasyMove),
            (k('F'), Modifiers::ALT, Command::OpenMainMenu),
            (k('I'), Modifiers::ALT, Command::AddBookmark),
            (
                k('I'),
                Modifiers::CTRL.union(Modifiers::ALT),
                Command::OpenBookmarkEditor,
            ),
            (
                k('J'),
                Modifiers::NONE,
                Command::OpenJump {
                    target: JumpTarget::File,
                },
            ),
            (
                k('J'),
                Modifiers::CTRL,
                Command::OpenJump {
                    target: JumpTarget::Time,
                },
            ),
            (
                k('K'),
                Modifiers::CTRL,
                Command::OpenPrefsPage(PrefsPage::VisPlugin),
            ),
            (k('K'), Modifiers::ALT, Command::ConfigureVisPlugin),
            (
                k('K'),
                Modifiers::CTRL.union(Modifiers::SHIFT),
                Command::ToggleVisPlugin,
            ),
            (
                k('L'),
                Modifiers::NONE,
                Command::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: true,
                },
            ),
            (
                k('L'),
                Modifiers::CTRL,
                Command::OpenMedia {
                    kind: OpenMediaKind::Location,
                    play: true,
                },
            ),
            (
                k('L'),
                Modifiers::SHIFT,
                Command::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play: true,
                },
            ),
            (k('M'), Modifiers::ALT, Command::Minimize),
            (
                k('O'),
                Modifiers::ALT,
                Command::OpenPrefsPage(PrefsPage::VisOptions),
            ),
            (k('S'), Modifiers::ALT, Command::OpenSkinBrowser),
            (k('V'), Modifiers::CTRL, Command::ToggleStopAfterCurrent),
            (
                k('V'),
                Modifiers::ALT,
                Command::ToggleWindow(WindowId::Video),
            ),
        ];
        for (key, mods, expected) in cases {
            let mut m = MainWindow::new();
            assert_eq!(cmds(&mut m, key, mods), vec![expected], "{key:?} {mods:?}");
        }
    }

    #[test]
    fn the_remaining_new_keys_emit_their_commands() {
        let n = Modifiers::NONE;
        let c = Modifiers::CTRL;
        let cases: [(Key, Modifiers, Command); 8] = [
            (Key::F(1), n, Command::OpenAbout),
            (Key::F(4), c, Command::CloseFocusedWindow),
            (Key::F(5), n, Command::RefreshSkin),
            (Key::Numpad(1), n, Command::SkipBy { delta: -10 }),
            (Key::Numpad(3), n, Command::SkipBy { delta: 10 }),
            (Key::Tab, c, Command::FocusNextWindow),
            (k('W'), Modifiers::ALT, Command::ShowMainWindow),
            (k('Z'), c, Command::PlayIndex(0)),
        ];
        for (key, mods, expected) in cases {
            let mut m = MainWindow::new();
            assert_eq!(cmds(&mut m, key, mods), vec![expected], "{key:?} {mods:?}");
        }
    }

    // ----- volume -----

    fn machine_with_volume(percent: u8) -> MainWindow {
        MainWindow::from_state(crate::state::MainWindowState {
            volume: percent,
            ..Default::default()
        })
    }

    #[test]
    fn volume_step_is_the_measured_four() {
        // Measured on 2026-10-15 (RFC 0005 §8, O10 settled).
        assert_eq!(VOLUME_STEP, 4);
        let base = core_volume(50);
        assert_eq!(base, 128, "50% is core volume 128");
        for key in [Key::Up, Key::Numpad(8)] {
            let mut m = machine_with_volume(50);
            assert_eq!(
                cmds(&mut m, key, Modifiers::NONE),
                vec![Command::SetVolume(132)],
                "one press is exactly +4 core units"
            );
            assert!(m.state().volume > 50);
        }
        for key in [Key::Down, Key::Numpad(2)] {
            let mut m = machine_with_volume(50);
            assert_eq!(
                cmds(&mut m, key, Modifiers::NONE),
                vec![Command::SetVolume(124)],
                "one press is exactly -4 core units"
            );
            assert!(m.state().volume < 50);
        }
    }

    #[test]
    fn volume_keys_clamp_at_the_ends_with_a_partial_end_step() {
        // Saturation at 255/0, with a partial final step (the measurement saw
        // 253 -> 255 and 2 -> 0).
        let mut top = machine_with_volume(100);
        assert_eq!(
            cmds(&mut top, Key::Up, Modifiers::NONE),
            vec![Command::SetVolume(255)]
        );
        let mut bottom = machine_with_volume(0);
        assert_eq!(
            cmds(&mut bottom, Key::Down, Modifiers::NONE),
            vec![Command::SetVolume(0)]
        );
        // 99% is core volume 252: +4 would be 256, so the press lands on 255
        // (a partial step of 3).
        let mut near_top = machine_with_volume(99);
        assert_eq!(core_volume(99), 252);
        assert_eq!(
            cmds(&mut near_top, Key::Up, Modifiers::NONE),
            vec![Command::SetVolume(255)]
        );
        // 1% is core volume 3: -4 saturates at 0 (a partial step of 3).
        let mut near_bottom = machine_with_volume(1);
        assert_eq!(core_volume(1), 3);
        assert_eq!(
            cmds(&mut near_bottom, Key::Down, Modifiers::NONE),
            vec![Command::SetVolume(0)]
        );
    }

    #[test]
    fn repeated_presses_drift_because_the_machine_holds_a_percentage() {
        // Exposed by the RFC 0005 §8 measurement, not fixed here: the machine's
        // volume is the S02 display percentage, so `core_volume` re-quantises
        // between presses and the emitted deltas are 4, 5, 5, 3 ... instead of
        // exactly 4 each. Each press still applies the step to the core value it
        // reads; the state it reads is rounded. Reported as an open point.
        let mut m = machine_with_volume(0);
        let emitted: Vec<Command> = (0..4)
            .map(|_| cmds(&mut m, Key::Up, Modifiers::NONE))
            .map(|commands| commands[0].clone())
            .collect();
        assert_eq!(
            emitted,
            vec![
                Command::SetVolume(4),
                Command::SetVolume(9),
                Command::SetVolume(14),
                Command::SetVolume(17),
            ]
        );
    }

    #[test]
    fn repeated_presses_deliver_the_action_again() {
        let mut m = MainWindow::new();
        let first = cmds(&mut m, k('S'), Modifiers::NONE);
        let second = cmds(&mut m, k('S'), Modifiers::NONE);
        assert_eq!(first, second);
        assert!(!m.state().shuffle);
    }
}
