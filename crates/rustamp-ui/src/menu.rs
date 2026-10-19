// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup: a table-driven model, check-state mapping and renderer.
//!
//! **The live popup is the operating system's own menu** (wave122):
//! [`native_main_menu`] turns the table below into a
//! [`rustamp_platform::PopupRow`] tree and `rustamp-app` shows it with
//! `TrackPopupMenu`. That is the faithful answer - S04 §1 settles the
//! reference popup's window class as `#32768`, which is the class Windows
//! uses for menus - and it is what makes the menu a frameless popup in the
//! system font, with the separators, submenu arrows, right-aligned
//! accelerators and tick marks the OS draws itself.
//!
//! [`render_main_window`](crate::render::render_main_window)'s sibling
//! [`render_main_menu`] below is the **retired** self-drawn popup: a 323 x 569
//! surface in the `text.bmp` bitmap font. It is kept for its S04 geometry
//! receipts and its golden tests, and nothing in the player paints it any
//! more. It is where the unreadable text came from: the glyph sheet is a
//! small fixed-cell font and the measured rows are 35 px tall, so no scale of
//! it reads like a menu.
//!
//! The popup is the `#32768` window the menu button (and `Alt+F`, S03 §1 id
//! 40043) opens. Its client size (**323 x 569**) and the check-mark cell
//! (**x = 16-29**) are **settled** measurements from spec **S04** (audited
//! 2026-10-16; run `20261016-011612`):
//!
//! * popup inventory, client sizes and class: S04 §1, §8;
//! * the main menu's 15 rows in order and its 4 separator rules: S04 §2;
//! * the three checked rows and their bboxes: S04 §3.1;
//! * the six submenu arrows and their column: S04 §3.2, §8;
//! * the six `Alt+*` hints and their column: S04 §3.3;
//! * the `View file info` hint geometry without an asserted label: S04 §3.4.
//!
//! Provenance rules used in this module:
//!
//! * Row **labels and command ids** come from spec **S03**, whose §1 preamble
//!   states the action texts are taken from the program's own menu resources.
//!   Each row cites its S03 section and id, plus the S04 §2 row it matches.
//! * The **15-item / 4-separator / 6-submenu shape** is the S04 §2
//!   measurement. The earlier task-card shape (11 items built from the S03
//!   action texts, with `Playback`/`Open`/`Time elapsed`/`Time remaining`
//!   groupings) was provisional and is replaced below row by row.
//! * **Submenu contents** stay provisional: S04 §9 states no submenu was ever
//!   opened (two-`#32768` refusal), so the entry arrays keep their S03-backed
//!   actions grouped under our own headings until S04 settles them.
//! * The **row bands** are a provisional split, not a measurement: 15 item
//!   rows of 35 px and 4 separator rows of 11 px fill 323 x 569 exactly
//!   (`15 * 35 + 4 * 11 = 569`). S04 §8 leaves item height, separator
//!   height and padding open, but the split is consistent with every measured
//!   y value (separator rules at y = 40/156/377/528, the §3.1 check bboxes
//!   and the §3.2 arrow bands all fall inside their row's band).
//! * The **rendering** (face colour, label origin, separator etching, submenu
//!   arrow, check-mark strokes) is our own look-alike drawing, documented
//!   per item. It is not claimed pixel-exact: the reference popup is a native
//!   menu, not a skinned surface. Measured placements the renderer honours:
//!   check cell x = 16-29 (S04 §3.1), arrow column x = 297-303 (S04 §3.2),
//!   hint column x ≈ 239-293 (S04 §3.3), separator rules 303 px wide
//!   (S04 §2), label text zone x ≈ 45-235 (S04 §3.5).
//!
//! Every leaf row maps to a [`ShortcutAction`](crate::shortcuts::ShortcutAction),
//! so activating a menu item runs exactly the same dispatch as pressing the
//! corresponding key (see [`crate::shortcuts`]). Rows the spec measures but
//! S03 gives no command (the title row, `Library`, `Exit`; S04 §2 notes `Exit`
//! has no S03 row) carry [`MenuAction::NoAction`]: S04 §9 explicitly does not
//! cover click-through, so no behaviour is invented for them. `Minibrowser`
//! likewise carries no dispatch: its action (`Alt+T`, S03 §1 id 40298) is
//! deferred with no window bridge yet (RFC 0005 §11) and the dispatcher drops
//! it.
//!
//! The elapsed/remaining toggle (S03 §1 id 40242) stays UI-local per RFC 0005
//! §9, but its rows live in the **Options popup** (S04 §4.1), not on the main
//! menu (S04 §2): [`MenuCheck::TimeElapsed`] / [`MenuCheck::TimeRemaining`]
//! still resolve from [`MainWindowState::time_remaining`](crate::state::MainWindowState)
//! for that popup's future use.

use std::ops::Range;

use rustamp_gfx::{BitmapFont, GlyphMap, Image, Rect};
use rustamp_platform::PopupRow;
use rustamp_skin::{Sheet, Skin, webamp};

use crate::shortcuts::ShortcutAction;
use crate::state::MainWindowState;

/// Window class of the menu popup (settled: S04 §1, every popup).
pub const MAIN_MENU_CLASS: &str = "#32768";

/// Client size of the main-menu popup (settled: S04 §1 row 1, §8).
pub const MAIN_MENU_CLIENT_SIZE: (u32, u32) = (323, 569);

/// Horizontal cell the check mark lives in (settled: S04 §3.1, §8 — the
/// checked/unchecked difference is confined to x = 16-29).
pub const CHECK_CELL_X: Range<u32> = 16..30;

/// How many rows the main menu has, separators included (settled: 15 item
/// rows per S04 §2 plus 4 separator rules).
pub const MAIN_MENU_ROWS: usize = 19;
/// How many of the rows are separators (settled: S04 §2, rules at y = 40,
/// 156, 377 and 528).
pub const MAIN_MENU_SEPARATORS: usize = 4;
/// How many rows carry a submenu arrow (settled: S04 §3.2 — `Play`,
/// `Bookmarks`, `Options`, `Playback`, `Visualization`, `Skins`).
pub const MAIN_MENU_SUBMENUS: usize = 6;

/// Provisional item-row height: fills 569 with
/// [`MAIN_MENU_SEPARATOR_HEIGHT_PX`] (see the module docs; S04 §8 leaves
/// item height open, but every measured y in §§2-3 falls inside its band).
pub const MAIN_MENU_ITEM_HEIGHT_PX: u32 = 35;
/// Provisional separator-row height (see the module docs, S04 §8 open).
pub const MAIN_MENU_SEPARATOR_HEIGHT_PX: u32 = 11;

/// Label origin: the left edge of the measured text zone (S04 §3.5 scans
/// text in x = 45-235; the exact origin is provisional, S04 §8 open).
const LABEL_X: i32 = 45;
/// Right edge of the shortcut-hint column (measured hint column x ≈ 239-293,
/// S04 §3.3; exact alignment is provisional — our bitmap font is not the
/// system menu font).
const HINT_RIGHT_X: i32 = 293;
/// Submenu-arrow column (settled: x = 297-303, S04 §3.2, §8).
const ARROW_X: i32 = 297;

/// What can carry a check mark.
///
/// Which rows show these is settled per popup: the main menu shows only
/// [`MenuCheck::MainWindow`], [`MenuCheck::PlaylistEditor`] and
/// [`MenuCheck::Equalizer`] (S04 §3.1); the `TimeElapsed` / `TimeRemaining`
/// pair and [`MenuCheck::EasyMove`] belong to the Options popup (S04 §4.1);
/// the remaining flags back the provisional submenu entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuCheck {
    /// Never checked.
    None,
    /// The main window is open (app state: it exists until quit).
    MainWindow,
    /// The playlist editor is open (`MainWindowState::playlist_open`).
    PlaylistEditor,
    /// The equalizer window is open (`MainWindowState::eq_open`).
    Equalizer,
    /// Elapsed time is shown (`!time_remaining`).
    TimeElapsed,
    /// Remaining time is shown (`time_remaining`).
    TimeRemaining,
    /// Always-on-top is engaged (app state).
    AlwaysOnTop,
    /// Easy-move is engaged (app state).
    EasyMove,
    /// Double-size is engaged (`MainWindowState::double_size`).
    DoubleSize,
    /// Windowshade is engaged (`MainWindowState::windowshade`).
    WindowShade,
    /// Stop-after-current is armed (engine state, via the app).
    StopAfterCurrent,
}

/// The app-owned flags a [`MenuCheck`] can read.
///
/// `rustamp-ui` never sees the app, so the app fills this from its own
/// state (quit flag, mode flags) and the engine snapshot; everything else
/// comes from [`MainWindowState`]. No check is hard-coded: each one resolves
/// from live state through [`check_state`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuFlags {
    /// The main window exists (false once the app quits).
    pub main_window_open: bool,
    /// Always-on-top is engaged.
    pub always_on_top: bool,
    /// Easy-move is engaged.
    pub easy_move: bool,
    /// Stop-after-current is armed.
    pub stop_after_current: bool,
}

impl Default for MenuFlags {
    fn default() -> Self {
        Self {
            main_window_open: true,
            always_on_top: false,
            easy_move: false,
            stop_after_current: false,
        }
    }
}

/// Resolves a [`MenuCheck`] against live state.
pub fn check_state(check: MenuCheck, state: &MainWindowState, flags: &MenuFlags) -> bool {
    match check {
        MenuCheck::None => false,
        MenuCheck::MainWindow => flags.main_window_open,
        MenuCheck::PlaylistEditor => state.playlist_open,
        MenuCheck::Equalizer => state.eq_open,
        MenuCheck::TimeElapsed => !state.time_remaining,
        MenuCheck::TimeRemaining => state.time_remaining,
        MenuCheck::AlwaysOnTop => flags.always_on_top,
        MenuCheck::EasyMove => flags.easy_move,
        MenuCheck::DoubleSize => state.double_size,
        MenuCheck::WindowShade => state.windowshade,
        MenuCheck::StopAfterCurrent => flags.stop_after_current,
    }
}

/// What activating a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// A leaf: runs the key-dispatch for this action (transport, toggles,
    /// dialogs, and the UI-local time flip alike).
    Shortcut(ShortcutAction),
    /// A parent: opens the submenu.
    Submenu(SubmenuId),
    /// `Exit`: quits the player (`Command::Quit`). S04 §2 notes `Exit` has no
    /// S03 row, so it has no shortcut; the app submits the quit command itself.
    Exit,
    /// A measured row with no dispatch behind it in this tree: the title row
    /// and `Library` have no S03 command, and `Minibrowser`'s action is
    /// deferred with no bridge yet (RFC 0005 §11). S04 §9 explicitly does not
    /// cover click-through, so no behaviour is invented: the row draws its
    /// label and never dispatches.
    NoAction,
}

/// Identity of a top-level row (settled order and labels: S04 §2 rows 1-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainMenuId {
    /// Title row `Rustamp...` (S04 §2 row 1; visual reading).
    Title,
    /// `Play` submenu (S04 §2 row 2).
    Play,
    /// `View file info` leaf (S04 §2 row 3; S03 §1 id 40188).
    ViewFileInfo,
    /// `Bookmarks` submenu (S04 §2 row 4).
    Bookmarks,
    /// Checkable: the main window is open (S04 §2 row 5, §3.1).
    MainWindow,
    /// Checkable: the playlist editor is open (S04 §2 row 6, §3.1).
    PlaylistEditor,
    /// Checkable: the equalizer window is open (S04 §2 row 7, §3.1).
    Equalizer,
    /// `Library` (S04 §2 row 8; hint `Alt+L`, S04 §3.3; no S03 command).
    Library,
    /// `Minibrowser` (S04 §2 row 9; hint `Alt+T`, S04 §3.3; S03 §1 id 40298).
    Minibrowser,
    /// `Video` (S04 §2 row 10; hint `Alt+V`, S04 §3.3; S03 §1 id 40328).
    Video,
    /// `Options` submenu (S04 §2 row 11).
    Options,
    /// `Playback` submenu (S04 §2 row 12).
    Playback,
    /// `Visualization` submenu (S04 §2 row 13).
    Visualization,
    /// `Skins` submenu (S04 §2 row 14).
    Skins,
    /// `Exit` (S04 §2 row 15; no S03 row).
    Exit,
}

/// One top-level row of the main menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainMenuItem {
    /// Stable identity.
    pub id: MainMenuId,
    /// Row label: the S04 §2 visual reading for main-menu rows (S03 action
    /// text where the row has a command).
    pub label: &'static str,
    /// Key hint shown right-aligned (settled S04 §3.3 readings on the six
    /// window rows; the `View file info` hint is provisional — S04 §3.4
    /// records its geometry but asserts no label, so the S03 §1 key stands
    /// in, marked as such).
    pub shortcut: Option<&'static str>,
    /// Check state source (settled: only the three window rows, S04 §3.1).
    pub check: MenuCheck,
    /// What activation does (S04 §9: click-through unmeasured — leaf actions
    /// are the pre-existing key dispatches, nothing new).
    pub action: MenuAction,
}

/// One row of [`MAIN_MENU`]: an item or a separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRow {
    /// A labelled row.
    Item(MainMenuItem),
    /// A separator row.
    Separator,
}

/// One entry of a submenu (submenus nest no further: no evidence for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmenuItem {
    /// Entry label (S03 action text).
    pub label: &'static str,
    /// Key hint shown right-aligned (S03 key).
    pub shortcut: Option<&'static str>,
    /// Check state source.
    pub check: MenuCheck,
    /// What activation does (leaves only).
    pub action: ShortcutAction,
}

/// Identity of a submenu (one per arrow row, in row order: S04 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubmenuId {
    /// The `Play` submenu (S04 §2 row 2; contents provisional, S04 §9).
    Play,
    /// Transport and play order (S04 §2 row 12; contents provisional, S04 §9).
    Playback,
    /// Bookmarks (S04 §2 row 4; contents provisional, S04 §9).
    Bookmarks,
    /// Windows, modes and dialogs (S04 §2 row 11; contents provisional, S04 §9).
    Options,
    /// Visualization (S04 §2 row 13; contents provisional, S04 §9).
    Visualization,
    /// Skins (S04 §2 row 14; contents provisional, S04 §9).
    Skins,
}

/// The main menu, top to bottom: 15 items and 4 separators (settled: S04 §2).
///
/// Every row cites the S04 §2 row it matches. Leaf actions are the
/// pre-existing key dispatches (S04 §9 covers no click-through, so nothing
/// new is added); submenu contents stay provisional (S04 §9).
pub const MAIN_MENU: [MenuRow; MAIN_MENU_ROWS] = [
    // S04 §2 row 1: title row (visual reading, y≈11-28).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Title,
        label: "Rustamp...",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::NoAction,
    }),
    // S04 §2 separator at y=40.
    MenuRow::Separator,
    // S04 §2 row 2: `Play` submenu (arrow S04 §3.2, y 57-68).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Play,
        label: "Play",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Play),
    }),
    // S04 §2 row 3: `View file info` (S03 §1 id 40188; hint provisional,
    // S04 §3.4).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::ViewFileInfo,
        label: "View file info",
        shortcut: Some("Alt+3"),
        check: MenuCheck::None,
        action: MenuAction::Shortcut(ShortcutAction::ViewFileInfo),
    }),
    // S04 §2 row 4: `Bookmarks` submenu (arrow S04 §3.2, y 127-138).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Bookmarks,
        label: "Bookmarks",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Bookmarks),
    }),
    // S04 §2 separator at y=156.
    MenuRow::Separator,
    // S04 §2 row 5: `Main Window` (S03 §1 id 40258; hint S04 §3.3; check
    // bbox 16,174-29,183 per S04 §3.1).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::MainWindow,
        label: "Main Window",
        shortcut: Some("Alt+W"),
        check: MenuCheck::MainWindow,
        action: MenuAction::Shortcut(ShortcutAction::ShowMainWindow),
    }),
    // S04 §2 row 6: `Playlist Editor` (S03 §1 id 40040; hint S04 §3.3;
    // check bbox y 209-218 per S04 §3.1).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::PlaylistEditor,
        label: "Playlist Editor",
        shortcut: Some("Alt+E"),
        check: MenuCheck::PlaylistEditor,
        action: MenuAction::Shortcut(ShortcutAction::TogglePlaylistEditor),
    }),
    // S04 §2 row 7: `Equalizer` (S03 §1 id 40036; hint S04 §3.3; check bbox
    // y 244-253 per S04 §3.1).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Equalizer,
        label: "Equalizer",
        shortcut: Some("Alt+G"),
        check: MenuCheck::Equalizer,
        action: MenuAction::Shortcut(ShortcutAction::ToggleEqualizer),
    }),
    // S04 §2 row 8: `Library` (hint `Alt+L`, S04 §3.3; no S03 command, so
    // no dispatch — S04 §9).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Library,
        label: "Library",
        shortcut: Some("Alt+L"),
        check: MenuCheck::None,
        action: MenuAction::NoAction,
    }),
    // S04 §2 row 9: `Minibrowser` (hint `Alt+T`, S04 §3.3; S03 §1 id 40298,
    // but the dispatcher drops it — RFC 0005 §11 — so no dispatch).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Minibrowser,
        label: "Minibrowser",
        shortcut: Some("Alt+T"),
        check: MenuCheck::None,
        action: MenuAction::NoAction,
    }),
    // S04 §2 row 10: `Video` (hint `Alt+V`, S04 §3.3; S03 §1 id 40328).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Video,
        label: "Video",
        shortcut: Some("Alt+V"),
        check: MenuCheck::None,
        action: MenuAction::Shortcut(ShortcutAction::ToggleVideoWindow),
    }),
    // S04 §2 separator at y=377.
    MenuRow::Separator,
    // S04 §2 row 11: `Options` submenu (arrow S04 §3.2, y 394-405).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Options,
        label: "Options",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Options),
    }),
    // S04 §2 row 12: `Playback` submenu (arrow S04 §3.2, y 429-440).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Playback,
        label: "Playback",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Playback),
    }),
    // S04 §2 row 13: `Visualization` submenu (arrow S04 §3.2, y 464-475).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Visualization,
        label: "Visualization",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Visualization),
    }),
    // S04 §2 row 14: `Skins` submenu (arrow S04 §3.2, y 499-510).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Skins,
        label: "Skins",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Skins),
    }),
    // S04 §2 separator at y=528.
    MenuRow::Separator,
    // S04 §2 row 15: `Exit` (no S03 row; it quits the player).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Exit,
        label: "Exit",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Exit,
    }),
];

/// Transport entries shown under the `Playback` submenu.
///
/// Provisional grouping (see [`PLAY_ENTRIES`]): S04 §9 leaves submenu
/// contents unmeasured; the actions are S03 §1 (ids 40044-40048, 40154,
/// 40195, 40197).
const PLAYBACK_ENTRIES: [SubmenuItem; 8] = [
    SubmenuItem {
        // S03 §1 id 40045.
        label: "Play",
        shortcut: Some("X"),
        check: MenuCheck::None,
        action: ShortcutAction::Play,
    },
    SubmenuItem {
        // S03 §1 id 40046.
        label: "Pause / unpause",
        shortcut: Some("C"),
        check: MenuCheck::None,
        action: ShortcutAction::PauseUnpause,
    },
    SubmenuItem {
        // S03 §1 id 40047.
        label: "Stop",
        shortcut: Some("V"),
        check: MenuCheck::None,
        action: ShortcutAction::Stop,
    },
    SubmenuItem {
        // S03 §1 id 40044.
        label: "Previous track",
        shortcut: Some("Z"),
        check: MenuCheck::None,
        action: ShortcutAction::PreviousTrack,
    },
    SubmenuItem {
        // S03 §1 id 40048.
        label: "Next track",
        shortcut: Some("B"),
        check: MenuCheck::None,
        action: ShortcutAction::NextTrack,
    },
    SubmenuItem {
        // S03 §1 id 40197.
        label: "10 tracks back",
        shortcut: Some("Num1"),
        check: MenuCheck::None,
        action: ShortcutAction::TenTracksBack,
    },
    SubmenuItem {
        // S03 §1 id 40195.
        label: "10 tracks forward",
        shortcut: Some("Num3"),
        check: MenuCheck::None,
        action: ShortcutAction::TenTracksForward,
    },
    SubmenuItem {
        // S03 §1 id 40154 (RFC 0005 O1: `PlayIndex(0)`).
        label: "Start of list",
        shortcut: Some("Ctrl+Z"),
        check: MenuCheck::None,
        action: ShortcutAction::StartOfList,
    },
];

/// File-access entries shown under the `Play` submenu.
///
/// Provisional grouping: S04 §9 states no submenu was ever opened, so which
/// entries the measured `Play` submenu holds is unmeasured. The actions
/// themselves are S03-backed (ids cited per entry); only the placement under
/// `Play` is ours.
const PLAY_ENTRIES: [SubmenuItem; 6] = [
    SubmenuItem {
        // S03 §1 id 40029.
        label: "Open file(s)",
        shortcut: Some("L"),
        check: MenuCheck::None,
        action: ShortcutAction::OpenFiles,
    },
    SubmenuItem {
        // S03 §1 id 40185.
        label: "Open location",
        shortcut: Some("Ctrl+L"),
        check: MenuCheck::None,
        action: ShortcutAction::OpenLocation,
    },
    SubmenuItem {
        // S03 §1 id 40187.
        label: "Open directory",
        shortcut: Some("Shift+L"),
        check: MenuCheck::None,
        action: ShortcutAction::OpenDirectory,
    },
    SubmenuItem {
        // S03 §1 id 40194.
        label: "Jump to file",
        shortcut: Some("J"),
        check: MenuCheck::None,
        action: ShortcutAction::JumpToFile,
    },
    SubmenuItem {
        // S03 §1 id 40193.
        label: "Jump to time",
        shortcut: Some("Ctrl+J"),
        check: MenuCheck::None,
        action: ShortcutAction::JumpToTime,
    },
    SubmenuItem {
        // S03 §1 id 40188.
        label: "View file info",
        shortcut: Some("Alt+3"),
        check: MenuCheck::None,
        action: ShortcutAction::ViewFileInfo,
    },
];

/// Bookmark entries shown under the `Bookmarks` submenu.
///
/// Provisional grouping (see [`PLAY_ENTRIES`]): S03 §1 ids 40320, 40321.
const BOOKMARKS_ENTRIES: [SubmenuItem; 2] = [
    SubmenuItem {
        // S03 §1 id 40321.
        label: "Add current track as a bookmark",
        shortcut: Some("Alt+I"),
        check: MenuCheck::None,
        action: ShortcutAction::AddBookmark,
    },
    SubmenuItem {
        // S03 §1 id 40320.
        label: "Edit bookmarks",
        shortcut: Some("Ctrl+Alt+I"),
        check: MenuCheck::None,
        action: ShortcutAction::EditBookmarks,
    },
];

/// Window, mode and dialog entries shown under the `Options` submenu.
///
/// Provisional grouping (see [`PLAY_ENTRIES`]): S03 §1 window/mode/dialog
/// block (ids 40012, 40019, 40063, 40064, 40157, 40186, 40224, 40258, 40291,
/// 40334, 40041).
const OPTIONS_ENTRIES: [SubmenuItem; 11] = [
    SubmenuItem {
        // S03 §1 id 40012.
        label: "Preferences",
        shortcut: Some("Ctrl+P"),
        check: MenuCheck::None,
        action: ShortcutAction::Preferences,
    },
    SubmenuItem {
        // S03 §1 id 40019.
        label: "Toggle always on top",
        shortcut: Some("Ctrl+A"),
        check: MenuCheck::AlwaysOnTop,
        action: ShortcutAction::ToggleAlwaysOnTop,
    },
    SubmenuItem {
        // S03 §1 id 40165.
        label: "Toggle double size",
        shortcut: Some("Ctrl+D"),
        check: MenuCheck::DoubleSize,
        action: ShortcutAction::ToggleDoubleSize,
    },
    SubmenuItem {
        // S03 §1 id 40064.
        label: "Toggle windowshade",
        shortcut: Some("Ctrl+W"),
        check: MenuCheck::WindowShade,
        action: ShortcutAction::ToggleWindowShade,
    },
    SubmenuItem {
        // S03 §1 id 40186.
        label: "Toggle easy move",
        shortcut: Some("Ctrl+E"),
        check: MenuCheck::EasyMove,
        action: ShortcutAction::ToggleEasyMove,
    },
    SubmenuItem {
        // S03 §1 id 40157.
        label: "Stop after current track",
        shortcut: Some("Ctrl+V"),
        check: MenuCheck::StopAfterCurrent,
        action: ShortcutAction::StopAfterCurrent,
    },
    SubmenuItem {
        // S03 §1 id 40334 (provisional key).
        label: "Minimize",
        shortcut: Some("Alt+M"),
        check: MenuCheck::None,
        action: ShortcutAction::Minimize,
    },
    SubmenuItem {
        // S03 §1 id 40258.
        label: "Show the main window",
        shortcut: Some("Alt+W"),
        check: MenuCheck::None,
        action: ShortcutAction::ShowMainWindow,
    },
    SubmenuItem {
        // S03 §1 id 40224 (provisional key).
        label: "Close the focused window",
        shortcut: Some("Ctrl+F4"),
        check: MenuCheck::None,
        action: ShortcutAction::CloseFocusedWindow,
    },
    SubmenuItem {
        // S03 §1 id 40063 (provisional key).
        label: "Switch to the next window",
        shortcut: Some("Ctrl+Tab"),
        check: MenuCheck::None,
        action: ShortcutAction::NextWindow,
    },
    SubmenuItem {
        // S03 §1 id 40041.
        label: "About box",
        shortcut: Some("F1"),
        check: MenuCheck::None,
        action: ShortcutAction::About,
    },
];

/// Visualization entries shown under the `Visualization` submenu.
///
/// Provisional grouping (see [`PLAY_ENTRIES`]): S03 §1 ids 40191, 40192,
/// 40221, 40317.
const VISUALIZATION_ENTRIES: [SubmenuItem; 4] = [
    SubmenuItem {
        // S03 §1 id 40191.
        label: "Select visualization plug-in",
        shortcut: Some("Ctrl+K"),
        check: MenuCheck::None,
        action: ShortcutAction::SelectVisPlugin,
    },
    SubmenuItem {
        // S03 §1 id 40317.
        label: "Visualization options",
        shortcut: Some("Alt+O"),
        check: MenuCheck::None,
        action: ShortcutAction::VisOptions,
    },
    SubmenuItem {
        // S03 §1 id 40221.
        label: "Configure visualization plug-in",
        shortcut: Some("Alt+K"),
        check: MenuCheck::None,
        action: ShortcutAction::ConfigureVisPlugin,
    },
    SubmenuItem {
        // S03 §1 id 40192.
        label: "Start/stop visualization plug-in",
        shortcut: Some("Ctrl+Shift+K"),
        check: MenuCheck::None,
        action: ShortcutAction::ToggleVisPlugin,
    },
];

/// Skin entries shown under the `Skins` submenu.
///
/// Provisional grouping (see [`PLAY_ENTRIES`]): S03 §1 ids 40219, 40291.
const SKINS_ENTRIES: [SubmenuItem; 2] = [
    SubmenuItem {
        // S03 §1 id 40219.
        label: "Skin browser",
        shortcut: Some("Alt+S"),
        check: MenuCheck::None,
        action: ShortcutAction::SkinBrowser,
    },
    SubmenuItem {
        // S03 §1 id 40291 (provisional key).
        label: "Refresh skin",
        shortcut: Some("F5"),
        check: MenuCheck::None,
        action: ShortcutAction::RefreshSkin,
    },
];

/// The entries of a submenu, top to bottom.
///
/// Provisional throughout: S04 §9 states no submenu was ever opened, so the
/// grouping under each parent is ours; only the actions (S03 ids) are
/// spec-backed.
pub fn submenu_entries(id: SubmenuId) -> &'static [SubmenuItem] {
    match id {
        SubmenuId::Play => &PLAY_ENTRIES,
        SubmenuId::Playback => &PLAYBACK_ENTRIES,
        SubmenuId::Bookmarks => &BOOKMARKS_ENTRIES,
        SubmenuId::Options => &OPTIONS_ENTRIES,
        SubmenuId::Visualization => &VISUALIZATION_ENTRIES,
        SubmenuId::Skins => &SKINS_ENTRIES,
    }
}

/// The top-level item carrying `id`, if it is on the menu.
pub fn top_level(id: MainMenuId) -> Option<&'static MainMenuItem> {
    MAIN_MENU.iter().find_map(|row| match row {
        MenuRow::Item(item) if item.id == id => Some(item),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// The native popup menu
// ---------------------------------------------------------------------------

/// What one chosen menu entry does.
///
/// The native popup reports an id; this is what that id meant. Submenu
/// parents are **not** choices - the OS opens their submenu - so they never
/// appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuChoice {
    /// A leaf with a dispatchable action: runs the same key dispatch the
    /// matching shortcut runs (see [`crate::shortcuts`]).
    Action {
        /// The row's label, for the note an undispatchable action records.
        label: &'static str,
        /// The action to dispatch.
        action: ShortcutAction,
    },
    /// `Exit`: quits the player.
    Exit,
    /// A measured row with no dispatch behind it (the title row, `Library`,
    /// `Minibrowser`): choosing it closes the menu and does nothing,
    /// because S04 §9 covers no click-through for them.
    NoAction {
        /// The row's label.
        label: &'static str,
    },
}

impl MenuChoice {
    /// The row's label, whichever kind it is.
    pub fn label(self) -> &'static str {
        match self {
            MenuChoice::Action { label, .. } | MenuChoice::NoAction { label } => label,
            MenuChoice::Exit => "Exit",
        }
    }
}

/// The main menu as a native popup, with live check marks.
///
/// `rows` is the tree [`rustamp_platform::show_popup_menu`] draws: the OS
/// supplies the menu's appearance - the system font, the separator lines, the
/// submenu arrows, the right-aligned accelerator column and the tick marks -
/// because the reference's popups are system menus (S04 §1 settles their
/// window class as `#32768`). `choices` is the lookup for what came back:
/// an entry's id is its **1-based index** into it
/// ([`NativeMainMenu::choice_for_id`]).
///
/// What this carries from the model, unchanged: the S04 §2 row order and the
/// four separator positions, the six submenu parents (S04 §3.2), the six
/// `Alt+*` accelerators and the `View file info` key (S04 §3.3, §3.4), and
/// the three checkable window rows resolved against live state (S04 §3.1,
/// [`check_state`]). What it drops is only our own drawing: the face colour,
/// the hand-drawn arrows and ticks and the bitmap font, none of which a
/// system menu needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMainMenu {
    /// The row tree, in display order.
    pub rows: Vec<PopupRow>,
    /// What each id means; an id is the 1-based index into this.
    pub choices: Vec<MenuChoice>,
}

impl NativeMainMenu {
    /// What the chosen `id` means, or `None` for an id that is not ours
    /// (`0` is "dismissed").
    pub fn choice_for_id(&self, id: u32) -> Option<MenuChoice> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.choices.get(index).copied()
    }
}

/// Builds the main menu as a native popup tree for the current state.
///
/// Submenu entries are leaves (submenus nest no further - no evidence for
/// it), and their contents stay the provisional S03-backed grouping the model
/// already carries (S04 §9 never opened one). They are now reachable: the
/// self-drawn popup could only note that a submenu was deferred, because it
/// had no way to draw one.
pub fn native_main_menu(state: &MainWindowState, flags: &MenuFlags) -> NativeMainMenu {
    let mut rows = Vec::with_capacity(MAIN_MENU_ROWS);
    let mut choices = Vec::new();
    for row in MAIN_MENU.iter() {
        let item = match row {
            MenuRow::Separator => {
                rows.push(PopupRow::Separator);
                continue;
            }
            MenuRow::Item(item) => item,
        };
        match item.action {
            MenuAction::Submenu(id) => {
                let mut entries = Vec::new();
                for entry in submenu_entries(id) {
                    choices.push(MenuChoice::Action {
                        label: entry.label,
                        action: entry.action,
                    });
                    entries.push(PopupRow::Entry {
                        id: choices.len() as u32,
                        label: entry.label.to_string(),
                        accelerator: entry.shortcut.map(str::to_string),
                        checked: check_state(entry.check, state, flags),
                        enabled: true,
                    });
                }
                rows.push(PopupRow::Submenu {
                    label: item.label.to_string(),
                    rows: entries,
                });
            }
            MenuAction::Shortcut(action) => {
                choices.push(MenuChoice::Action {
                    label: item.label,
                    action,
                });
                rows.push(PopupRow::Entry {
                    id: choices.len() as u32,
                    label: item.label.to_string(),
                    accelerator: item.shortcut.map(str::to_string),
                    checked: check_state(item.check, state, flags),
                    enabled: true,
                });
            }
            MenuAction::Exit | MenuAction::NoAction => {
                choices.push(if item.action == MenuAction::Exit {
                    MenuChoice::Exit
                } else {
                    MenuChoice::NoAction { label: item.label }
                });
                rows.push(PopupRow::Entry {
                    id: choices.len() as u32,
                    label: item.label.to_string(),
                    accelerator: item.shortcut.map(str::to_string),
                    checked: check_state(item.check, state, flags),
                    enabled: true,
                });
            }
        }
    }
    NativeMainMenu { rows, choices }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The `(top, bottom)` pixel band of every top-level row under the
/// provisional 35/11 split (S04 §8 leaves the bands open; every measured y
/// in S04 §§2-3 falls inside its row's band — see the bands test).
pub fn row_bands() -> [(u32, u32); MAIN_MENU_ROWS] {
    let mut bands = [(0, 0); MAIN_MENU_ROWS];
    let mut y = 0;
    for (index, row) in MAIN_MENU.iter().enumerate() {
        let height = match row {
            MenuRow::Item(_) => MAIN_MENU_ITEM_HEIGHT_PX,
            MenuRow::Separator => MAIN_MENU_SEPARATOR_HEIGHT_PX,
        };
        bands[index] = (y, y + height);
        y += height;
    }
    bands
}

/// The top-level row index at client height `y`, if `y` is on the menu.
pub fn row_at_y(y: u32) -> Option<usize> {
    row_bands()
        .iter()
        .position(|(top, bottom)| y >= *top && y < *bottom)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Menu face background (our look-alike choice, not a measurement).
const MENU_FACE: [u8; 4] = [212, 208, 200, 255];
/// Separator etching: dark line, then a light line (our choice).
const SEPARATOR_DARK: [u8; 4] = [128, 128, 128, 255];
const SEPARATOR_LIGHT: [u8; 4] = [255, 255, 255, 255];
/// Check-mark, arrow and hint ink (our choice).
const MENU_INK: [u8; 4] = [0, 0, 0, 255];

/// The `text.bmp` bitmap font, or `None` when the skin has no Text sheet.
///
/// Same glyph map as the main-window renderer (S01 §2.2 via
/// `rustamp_skin::webamp`); labels are lowercased first because the classic
/// font has no uppercase glyphs.
fn menu_font(skin: &Skin) -> Option<BitmapFont> {
    let raster = skin.sheet(Sheet::Text)?;
    let image =
        Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).ok()?;
    let mut glyphs = GlyphMap::new(webamp::TEXT_CELL.0, webamp::TEXT_CELL.1);
    for (ch, col, row) in webamp::TEXT_GLYPHS {
        glyphs.insert(*ch, *col, *row);
    }
    Some(BitmapFont::new(image, glyphs, None, 0))
}

/// Draws one check mark centred in the check cell of an item band.
///
/// Our own drawing: two small strokes forming a tick. It stays inside the
/// measured [`CHECK_CELL_X`] (S04 §3.1); the 40-px measured glyph shape is
/// not reproduced — the reference popup is a native menu.
fn draw_check(canvas: &mut Image, band_top: u32) {
    let mid = band_top as i32 + (MAIN_MENU_ITEM_HEIGHT_PX / 2) as i32;
    // Short arm: (19, mid+1) to (23, mid+5).
    canvas.fill_rect(Rect::new(19, mid - 1, 3, 5), MENU_INK);
    canvas.fill_rect(Rect::new(21, mid + 1, 3, 5), MENU_INK);
    // Long arm: (23, mid+5) up to (29, mid-7).
    canvas.fill_rect(Rect::new(23, mid - 7, 3, 13), MENU_INK);
    canvas.fill_rect(Rect::new(25, mid - 7, 3, 10), MENU_INK);
}

/// Draws a right-pointing submenu arrow (our own drawing) inside the
/// measured arrow column x = 297-303, about 12 px tall (S04 §3.2).
fn draw_arrow(canvas: &mut Image, band_top: u32) {
    let mid = band_top as i32 + (MAIN_MENU_ITEM_HEIGHT_PX / 2) as i32;
    for (i, width) in [6, 6, 4, 4, 2, 2].iter().enumerate() {
        canvas.fill_rect(
            Rect::new(ARROW_X, mid - 6 + i as i32 * 2, *width, 2),
            MENU_INK,
        );
    }
}

/// Draws one item row: check mark, label, shortcut hint and submenu arrow.
fn draw_item(
    canvas: &mut Image,
    font: &Option<BitmapFont>,
    band_top: u32,
    label: &str,
    shortcut: Option<&str>,
    checked: bool,
    has_submenu: bool,
) {
    if checked {
        draw_check(canvas, band_top);
    }
    let baseline = band_top as i32 + ((MAIN_MENU_ITEM_HEIGHT_PX - webamp::TEXT_CELL.1) / 2) as i32;
    if let Some(font) = font {
        font.render_text(canvas, LABEL_X, baseline, &label.to_ascii_lowercase());
        if let Some(hint) = shortcut {
            let width = font.width_of(&hint.to_ascii_lowercase());
            font.render_text(
                canvas,
                HINT_RIGHT_X - width as i32,
                baseline,
                &hint.to_ascii_lowercase(),
            );
        }
    }
    if has_submenu {
        draw_arrow(canvas, band_top);
    }
}

/// Renders the main-menu popup at [`MAIN_MENU_CLIENT_SIZE`].
///
/// Check marks resolve through [`check_state`], which the caller feeds with
/// live state — the same state the window toggles write.
pub fn render_main_menu(skin: &Skin, state: &MainWindowState, flags: &MenuFlags) -> Image {
    let (width, height) = MAIN_MENU_CLIENT_SIZE;
    let mut canvas = Image::filled(width, height, MENU_FACE);
    let font = menu_font(skin);
    for (row, (top, bottom)) in MAIN_MENU.iter().zip(row_bands()) {
        match row {
            MenuRow::Separator => {
                // Measured 303-px full-width rule (S04 §2); the dark/light
                // etching itself is our choice.
                let mid = (top + bottom) / 2;
                canvas.fill_rect(Rect::new(10, mid as i32, 303, 1), SEPARATOR_DARK);
                canvas.fill_rect(Rect::new(10, mid as i32 + 1, 303, 1), SEPARATOR_LIGHT);
            }
            MenuRow::Item(item) => {
                let has_submenu = matches!(item.action, MenuAction::Submenu(_));
                draw_item(
                    &mut canvas,
                    &font,
                    top,
                    item.label,
                    item.shortcut,
                    check_state(item.check, state, flags),
                    has_submenu,
                );
            }
        }
    }
    canvas
}

/// Client size of a submenu popup: one item band per entry; the width fits
/// the longest label plus its hint.
///
/// Our layout helper only: S04 §9 states no submenu was ever opened, so
/// submenu sizes are unmeasured.
pub fn submenu_client_size(id: SubmenuId, skin: &Skin) -> (u32, u32) {
    let entries = submenu_entries(id);
    let height = entries.len() as u32 * MAIN_MENU_ITEM_HEIGHT_PX;
    let width = match menu_font(skin) {
        None => 240,
        Some(font) => {
            let content = entries
                .iter()
                .map(|entry| {
                    let label = font.width_of(&entry.label.to_ascii_lowercase());
                    let hint = entry
                        .shortcut
                        .map(|hint| font.width_of(&hint.to_ascii_lowercase()) + 16)
                        .unwrap_or(0);
                    LABEL_X as u32 + label + hint
                })
                .max()
                .unwrap_or(0);
            // Provisional right margin (submenu geometry unmeasured, S04 §9).
            content + 10 + 24
        }
    };
    (width, height)
}

/// Renders a submenu popup (same row look as [`render_main_menu`]).
pub fn render_submenu(
    skin: &Skin,
    id: SubmenuId,
    state: &MainWindowState,
    flags: &MenuFlags,
) -> Image {
    let (width, height) = submenu_client_size(id, skin);
    let mut canvas = Image::filled(width, height, MENU_FACE);
    let font = menu_font(skin);
    for (index, entry) in submenu_entries(id).iter().enumerate() {
        let top = index as u32 * MAIN_MENU_ITEM_HEIGHT_PX;
        draw_item(
            &mut canvas,
            &font,
            top,
            entry.label,
            entry.shortcut,
            check_state(entry.check, state, flags),
            false,
        );
    }
    canvas
}

// ---------------------------------------------------------------------------
// Popup placement and hit-testing
// ---------------------------------------------------------------------------

/// The client point whose click opens the main menu: the window-menu button
/// (settled: S04 §1 row 1, posted left click at client (6,3)).
pub const MENU_BUTTON_POINT: (i32, i32) = (6, 3);

/// Where the popup opens, given the main window's outer (screen) origin.
///
/// Own choice, stated plainly: S04 settles the opening click (client (6,3),
/// §1 row 1), the class (`#32768`, §1) and the client size (323 x 569, §§1,8)
/// but no screen position — popup placement is unmeasured (S04 §9), and the
/// non-client border is open (§8), so the client-exact offset under the
/// button cannot be computed. The popup's top-left anchors at the measured
/// click point, so it drops over the button it came from.
pub fn menu_popup_origin(main_outer: (i32, i32)) -> (i32, i32) {
    (
        main_outer.0 + MENU_BUTTON_POINT.0,
        main_outer.1 + MENU_BUTTON_POINT.1,
    )
}

/// The top-level item at popup-client `(x, y)`, if the point is on an item
/// row.
///
/// Separators and points outside the [`MAIN_MENU_CLIENT_SIZE`] client area
/// yield `None`: a separator click is not a selection (S04 §9 leaves
/// click-through unmeasured, so it is ignored, never activated). Dismissal on
/// a click outside the popup is the presenter's job — only it sees presses on
/// other windows — so out-of-bounds points are likewise `None` here.
pub fn menu_item_at(x: i32, y: i32) -> Option<&'static MainMenuItem> {
    let (width, height) = MAIN_MENU_CLIENT_SIZE;
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return None;
    }
    match &MAIN_MENU[row_at_y(y as u32)?] {
        MenuRow::Item(item) => Some(item),
        MenuRow::Separator => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behaviour::MainWindow;
    use crate::shortcuts::ShortcutOutcome;

    fn item_rows() -> Vec<MainMenuItem> {
        MAIN_MENU
            .iter()
            .filter_map(|row| match row {
                MenuRow::Item(item) => Some(*item),
                MenuRow::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_table_has_nineteen_rows_with_four_separators() {
        // Settled shape: 15 item rows (S04 §2 rows 1-15) plus 4 separator
        // rules (S04 §2, y = 40/156/377/528).
        assert_eq!(MAIN_MENU.len(), MAIN_MENU_ROWS);
        assert_eq!(MAIN_MENU.len(), 19);
        let separators = MAIN_MENU
            .iter()
            .filter(|row| matches!(row, MenuRow::Separator))
            .count();
        assert_eq!(separators, MAIN_MENU_SEPARATORS);
        assert_eq!(separators, 4);
        assert_eq!(item_rows().len(), 15);
    }

    #[test]
    fn the_items_match_the_measured_row_order() {
        // Settled labels top to bottom (S04 §2 table).
        let labels: Vec<&str> = item_rows().iter().map(|item| item.label).collect();
        assert_eq!(
            labels,
            [
                "Rustamp...",
                "Play",
                "View file info",
                "Bookmarks",
                "Main Window",
                "Playlist Editor",
                "Equalizer",
                "Library",
                "Minibrowser",
                "Video",
                "Options",
                "Playback",
                "Visualization",
                "Skins",
                "Exit",
            ]
        );
    }

    #[test]
    fn the_window_rows_carry_the_measured_alt_hints() {
        // Settled visual readings (S04 §3.3; the `View file info` hint is
        // provisional — S04 §3.4 asserts no label, so the S03 §1 key stands
        // in — and the submenu/title/`Exit` rows carry none).
        let hints: Vec<(MainMenuId, Option<&str>)> = item_rows()
            .iter()
            .map(|item| (item.id, item.shortcut))
            .collect();
        assert_eq!(
            hints,
            [
                (MainMenuId::Title, None),
                (MainMenuId::Play, None),
                (MainMenuId::ViewFileInfo, Some("Alt+3")),
                (MainMenuId::Bookmarks, None),
                (MainMenuId::MainWindow, Some("Alt+W")),
                (MainMenuId::PlaylistEditor, Some("Alt+E")),
                (MainMenuId::Equalizer, Some("Alt+G")),
                (MainMenuId::Library, Some("Alt+L")),
                (MainMenuId::Minibrowser, Some("Alt+T")),
                (MainMenuId::Video, Some("Alt+V")),
                (MainMenuId::Options, None),
                (MainMenuId::Playback, None),
                (MainMenuId::Visualization, None),
                (MainMenuId::Skins, None),
                (MainMenuId::Exit, None),
            ]
        );
    }

    #[test]
    fn separators_sit_between_the_measured_groups() {
        // S04 §2: after the title row, after `Bookmarks`, after `Video` and
        // after `Skins`.
        let at: Vec<usize> = MAIN_MENU
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                MenuRow::Separator => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(at, [1, 5, 12, 17]);
    }

    #[test]
    fn six_rows_carry_submenus_and_each_has_entries() {
        // Settled arrow set in row order (S04 §3.2); contents provisional
        // (S04 §9).
        let parents: Vec<(MainMenuId, SubmenuId)> = item_rows()
            .iter()
            .filter_map(|item| match item.action {
                MenuAction::Submenu(id) => Some((item.id, id)),
                _ => None,
            })
            .collect();
        assert_eq!(parents.len(), MAIN_MENU_SUBMENUS);
        assert_eq!(
            parents,
            [
                (MainMenuId::Play, SubmenuId::Play),
                (MainMenuId::Bookmarks, SubmenuId::Bookmarks),
                (MainMenuId::Options, SubmenuId::Options),
                (MainMenuId::Playback, SubmenuId::Playback),
                (MainMenuId::Visualization, SubmenuId::Visualization),
                (MainMenuId::Skins, SubmenuId::Skins),
            ]
        );
        for (_, id) in &parents {
            assert!(!submenu_entries(*id).is_empty(), "{id:?} has entries");
        }
    }

    #[test]
    fn the_checkable_set_is_the_three_window_rows() {
        // Settled: three checked rows (S04 §3.1); every other main-menu row
        // is unchecked.
        let checked: Vec<(MainMenuId, MenuCheck)> = item_rows()
            .iter()
            .filter(|item| item.check != MenuCheck::None)
            .map(|item| (item.id, item.check))
            .collect();
        assert_eq!(
            checked,
            [
                (MainMenuId::MainWindow, MenuCheck::MainWindow),
                (MainMenuId::PlaylistEditor, MenuCheck::PlaylistEditor),
                (MainMenuId::Equalizer, MenuCheck::Equalizer),
            ]
        );
        // The mode checks live in the Options submenu.
        let option_checks: Vec<MenuCheck> = submenu_entries(SubmenuId::Options)
            .iter()
            .filter(|entry| entry.check != MenuCheck::None)
            .map(|entry| entry.check)
            .collect();
        assert_eq!(
            option_checks,
            [
                MenuCheck::AlwaysOnTop,
                MenuCheck::DoubleSize,
                MenuCheck::WindowShade,
                MenuCheck::EasyMove,
                MenuCheck::StopAfterCurrent,
            ]
        );
    }

    #[test]
    fn every_dispatched_menu_action_runs_the_key_dispatch() {
        // No dispatched menu row may reference an action the dispatcher
        // drops: `Shortcut` leaves must yield commands (or a handled flip),
        // never `Unhandled`. [`MenuAction::NoAction`] rows (title, Library,
        // Minibrowser, Exit) draw only and are skipped here.
        let mut machine = MainWindow::default();
        let mut leaves = Vec::new();
        let mut undrawn = 0;
        for item in item_rows() {
            match item.action {
                MenuAction::Shortcut(action) => leaves.push((item.label, action)),
                MenuAction::Submenu(_) => {}
                MenuAction::NoAction => undrawn += 1,
                MenuAction::Exit => {}
            }
        }
        assert_eq!(undrawn, 3);
        for entry in [
            &PLAYBACK_ENTRIES[..],
            &PLAY_ENTRIES[..],
            &BOOKMARKS_ENTRIES[..],
            &OPTIONS_ENTRIES[..],
            &VISUALIZATION_ENTRIES[..],
            &SKINS_ENTRIES[..],
        ]
        .concat()
        {
            leaves.push((entry.label, entry.action));
        }
        assert_eq!(leaves.len(), 5 + 8 + 6 + 2 + 11 + 4 + 2);
        for (label, action) in leaves {
            let outcome = machine.dispatch_shortcut(action, 0);
            assert!(
                !matches!(outcome, ShortcutOutcome::Unhandled(_)),
                "{label} ({action:?}) has no command"
            );
        }
    }

    #[test]
    fn the_time_pair_resolves_oppositely_from_live_state() {
        // The elapsed/remaining rows live in the Options popup (S04 §4.1),
        // not on the main menu (S04 §2); the pair still resolves from
        // `time_remaining`, which the Options popup will read.
        let flags = MenuFlags::default();
        let mut state = MainWindowState::default();
        assert!(check_state(MenuCheck::TimeElapsed, &state, &flags));
        assert!(!check_state(MenuCheck::TimeRemaining, &state, &flags));
        state.time_remaining = true;
        assert!(!check_state(MenuCheck::TimeElapsed, &state, &flags));
        assert!(check_state(MenuCheck::TimeRemaining, &state, &flags));
    }

    #[test]
    fn checks_follow_live_state_not_hard_coded_values() {
        let flags = MenuFlags::default();
        let mut state = MainWindowState::default();
        assert!(!check_state(MenuCheck::PlaylistEditor, &state, &flags));
        assert!(!check_state(MenuCheck::Equalizer, &state, &flags));
        state.playlist_open = true;
        state.eq_open = true;
        state.double_size = true;
        state.windowshade = true;
        assert!(check_state(MenuCheck::PlaylistEditor, &state, &flags));
        assert!(check_state(MenuCheck::Equalizer, &state, &flags));
        assert!(check_state(MenuCheck::DoubleSize, &state, &flags));
        assert!(check_state(MenuCheck::WindowShade, &state, &flags));
        assert!(check_state(MenuCheck::MainWindow, &state, &flags));
        assert!(!check_state(
            MenuCheck::MainWindow,
            &state,
            &MenuFlags {
                main_window_open: false,
                ..flags
            }
        ));
        assert!(check_state(
            MenuCheck::AlwaysOnTop,
            &state,
            &MenuFlags {
                always_on_top: true,
                ..flags
            }
        ));
        assert!(check_state(
            MenuCheck::StopAfterCurrent,
            &state,
            &MenuFlags {
                stop_after_current: true,
                ..flags
            }
        ));
        assert!(!check_state(MenuCheck::None, &state, &flags));
    }

    #[test]
    fn the_window_toggles_map_to_the_toggle_commands() {
        assert_eq!(
            top_level(MainMenuId::PlaylistEditor).unwrap().action,
            MenuAction::Shortcut(ShortcutAction::TogglePlaylistEditor)
        );
        assert_eq!(
            top_level(MainMenuId::Equalizer).unwrap().action,
            MenuAction::Shortcut(ShortcutAction::ToggleEqualizer)
        );
        let mut machine = MainWindow::default();
        machine.dispatch_shortcut(ShortcutAction::TogglePlaylistEditor, 0);
        assert!(machine.state().playlist_open);
        machine.dispatch_shortcut(ShortcutAction::ToggleEqualizer, 0);
        assert!(machine.state().eq_open);
    }

    #[test]
    fn the_bands_fill_the_client_exactly() {
        let bands = row_bands();
        assert_eq!(bands.len(), MAIN_MENU_ROWS);
        assert_eq!(bands[0].0, 0);
        assert_eq!(bands[MAIN_MENU_ROWS - 1].1, MAIN_MENU_CLIENT_SIZE.1);
        for window in bands.windows(2) {
            assert_eq!(window[0].1, window[1].0);
        }
        for (row, (top, bottom)) in MAIN_MENU.iter().zip(bands) {
            let want = match row {
                MenuRow::Item(_) => MAIN_MENU_ITEM_HEIGHT_PX,
                MenuRow::Separator => MAIN_MENU_SEPARATOR_HEIGHT_PX,
            };
            assert_eq!(bottom - top, want);
        }
    }

    #[test]
    fn row_at_y_resolves_every_pixel_to_its_row() {
        assert_eq!(row_at_y(0), Some(0));
        assert_eq!(row_at_y(34), Some(0));
        assert_eq!(row_at_y(35), Some(1));
        assert_eq!(row_at_y(568), Some(18));
        assert_eq!(row_at_y(569), None);
        // Every band maps back to its own index.
        for (index, (top, bottom)) in row_bands().iter().enumerate() {
            assert_eq!(row_at_y(*top), Some(index));
            assert_eq!(row_at_y(bottom - 1), Some(index));
        }
    }

    #[test]
    fn the_provisional_bands_contain_the_measured_marks() {
        // The 35/11 split is provisional (S04 §8 open), but it must stay
        // consistent with every measured y: the separator rules (S04 §2),
        // the check bboxes (S04 §3.1) and the arrow bands (S04 §3.2).
        let bands = row_bands();
        let contains = |index: usize, y: u32| {
            let (top, bottom) = bands[index];
            assert!(
                y >= top && y < bottom,
                "measured y={y} falls outside band {index} ({top}..{bottom})"
            );
        };
        // Separator rules at y = 40/156/377/528 sit in bands 1/5/12/17.
        for (index, y) in [(1, 40), (5, 156), (12, 377), (17, 528)] {
            contains(index, y);
        }
        // Check bboxes: Main Window 174-183, Playlist Editor 209-218,
        // Equalizer 244-253.
        for y in [174, 183] {
            contains(6, y);
        }
        for y in [209, 218] {
            contains(7, y);
        }
        for y in [244, 253] {
            contains(8, y);
        }
        // Arrow bands on the six submenu rows.
        for (index, top, bottom) in [
            (2, 57, 68),
            (4, 127, 138),
            (13, 394, 405),
            (14, 429, 440),
            (15, 464, 475),
            (16, 499, 510),
        ] {
            contains(index, top);
            contains(index, bottom - 1);
        }
        // Title row text zone (y≈11-28, S04 §2 row 1).
        contains(0, 11);
        contains(0, 27);
    }

    #[test]
    fn the_popup_anchors_at_the_measured_button_point() {
        // The opening click is client (6,3) (S04 §1 row 1); placement beyond
        // that anchor is unmeasured (S04 §9), so the origin is exactly the
        // main outer origin plus the button point.
        assert_eq!(MENU_BUTTON_POINT, (6, 3));
        assert_eq!(menu_popup_origin((100, 200)), (106, 203));
        assert_eq!(menu_popup_origin((0, 0)), (6, 3));
    }

    #[test]
    fn the_popup_size_and_class_are_the_measured_values() {
        // Settled: 323 x 569 (S04 §1 row 1, §8), class `#32768` (S04 §1).
        assert_eq!(MAIN_MENU_CLIENT_SIZE, (323, 569));
        assert_eq!(MAIN_MENU_CLASS, "#32768");
    }

    #[test]
    fn every_item_band_hits_its_row_and_separators_hit_nothing() {
        // Each item band's middle resolves to its own row; every separator
        // band resolves to `None` (not a selection).
        for (index, row) in MAIN_MENU.iter().enumerate() {
            let (top, bottom) = row_bands()[index];
            let middle = ((top + bottom) / 2) as i32;
            match row {
                MenuRow::Item(item) => {
                    assert_eq!(menu_item_at(10, middle), Some(item));
                    assert_eq!(menu_item_at(322, middle), Some(item));
                }
                MenuRow::Separator => {
                    assert_eq!(menu_item_at(10, middle), None);
                }
            }
        }
    }

    #[test]
    fn points_outside_the_client_hit_nothing() {
        assert_eq!(menu_item_at(-1, 10), None);
        assert_eq!(menu_item_at(10, -1), None);
        assert_eq!(menu_item_at(323, 10), None);
        assert_eq!(menu_item_at(10, 569), None);
        assert_eq!(menu_item_at(500, 500), None);
    }
}
