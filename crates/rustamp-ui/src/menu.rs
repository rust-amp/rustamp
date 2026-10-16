// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup: a table-driven model, check-state mapping and renderer.
//!
//! The popup is the `#32768` window the menu button (and `Alt+F`, S03 §1 id
//! 40043) opens. Its client size (**323 x 569**) and the check-mark cell
//! (**x = 16-29**) are lane task-card measurements that still await the S04
//! spec export (`specs/S04-*` does not exist yet; the spec task card is
//! `docs/tasks/S04-menus.md`). Every value taken from the task card is marked
//! as such below; everything else cites the audited spec or RFC it comes
//! from.
//!
//! Provenance rules used in this module:
//!
//! * Row **labels and command ids** come from spec **S03**, whose §1 preamble
//!   states the action texts are taken from the program's own menu resources.
//!   Each row cites its S03 section and id.
//! * The **15-row / 4-separator / 6-submenu shape** and the **checkable set**
//!   (Main Window, Playlist Editor, Equalizer, plus the Time elapsed / Time
//!   remaining pair) are the task-card measurement summary.
//! * The **row order, the separator positions, the submenu grouping and the
//!   group labels** (`Playback`, `Open`, …) are provisional: S03 records
//!   actions, not menu trees, so the grouping is ours until S04 settles it.
//!   The table is kept in one place precisely so it can be compared with the
//!   reading line by line.
//! * The **row bands** are a provisional split, not a measurement: 11 item
//!   rows of 47 px and 4 separator rows of 13 px fill 323 x 569 exactly
//!   (`11 * 47 + 4 * 13 = 569`). S04 must replace them with the measured
//!   bands.
//! * The **rendering** (face colour, label origin, separator etching, submenu
//!   arrow, check-mark strokes) is our own look-alike drawing, documented
//!   per item. It is not claimed pixel-exact: the reference popup is a native
//!   menu, not a skinned surface.
//!
//! Every leaf row maps to a [`ShortcutAction`](crate::shortcuts::ShortcutAction),
//! so activating a menu item runs exactly the same dispatch as pressing the
//! corresponding key (see [`crate::shortcuts`]). The elapsed/remaining toggle
//! (S03 §1 id 40242) stays UI-local per RFC 0005 §9: both time rows map to
//! `ShortcutAction::ToggleTimeRemaining`, which flips
//! [`MainWindowState::time_remaining`](crate::state::MainWindowState) with no
//! core command.

use std::ops::Range;

use rustamp_gfx::{BitmapFont, GlyphMap, Image, Rect};
use rustamp_skin::{Sheet, Skin, webamp};

use crate::shortcuts::ShortcutAction;
use crate::state::MainWindowState;

/// Window class of the menu popup (task-card measurement, pending S04).
pub const MAIN_MENU_CLASS: &str = "#32768";

/// Client size of the main-menu popup (task-card measurement, pending S04).
pub const MAIN_MENU_CLIENT_SIZE: (u32, u32) = (323, 569);

/// Horizontal cell the check mark lives in (task-card measurement: the
/// checked/unchecked difference is confined to x = 16-29, pending S04).
pub const CHECK_CELL_X: Range<u32> = 16..30;

/// How many rows the main menu has, separators included (task card).
pub const MAIN_MENU_ROWS: usize = 15;
/// How many of the rows are separators (task card).
pub const MAIN_MENU_SEPARATORS: usize = 4;
/// How many rows carry a submenu arrow (task card).
pub const MAIN_MENU_SUBMENUS: usize = 6;

/// Provisional item-row height (see the module docs: fills 569 with
/// [`MAIN_MENU_SEPARATOR_HEIGHT_PX`], pending S04).
pub const MAIN_MENU_ITEM_HEIGHT_PX: u32 = 47;
/// Provisional separator-row height (see the module docs, pending S04).
pub const MAIN_MENU_SEPARATOR_HEIGHT_PX: u32 = 13;

/// Label origin: right of the check cell plus a margin (our choice).
const LABEL_X: i32 = 40;
/// Right margin for the shortcut hints (our choice).
const HINT_MARGIN_RIGHT: u32 = 10;
/// Submenu-arrow column (our choice).
const ARROW_X: i32 = 304;

/// What can carry a check mark.
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
}

/// Identity of a top-level row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainMenuId {
    /// Transport and play-order submenu (provisional grouping).
    Playback,
    /// File-access submenu (provisional grouping).
    Open,
    /// Bookmark submenu (provisional grouping).
    Bookmarks,
    /// Window, mode and dialog submenu (provisional grouping).
    Options,
    /// Visualization submenu (provisional grouping).
    Visualization,
    /// Skin submenu (provisional grouping).
    Skins,
    /// Checkable: the main window is open.
    MainWindow,
    /// Checkable: the playlist editor is open.
    PlaylistEditor,
    /// Checkable: the equalizer window is open.
    Equalizer,
    /// Checkable: elapsed time is shown.
    TimeElapsed,
    /// Checkable: remaining time is shown.
    TimeRemaining,
}

/// One top-level row of the main menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainMenuItem {
    /// Stable identity.
    pub id: MainMenuId,
    /// Row label (S03 action text for leaves, task-card name for the time
    /// pair, our group name for parents).
    pub label: &'static str,
    /// Key hint shown right-aligned (S03 key, where the row has one).
    pub shortcut: Option<&'static str>,
    /// Check state source.
    pub check: MenuCheck,
    /// What activation does.
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

/// Identity of a submenu (one per arrow row, in row order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubmenuId {
    /// Transport and play order.
    Playback,
    /// File access.
    Open,
    /// Bookmarks.
    Bookmarks,
    /// Windows, modes and dialogs.
    Options,
    /// Visualization.
    Visualization,
    /// Skins.
    Skins,
}

/// The main menu, top to bottom: 11 items and 4 separators (task card).
///
/// Order, separator positions and grouping are provisional pending S04; the
/// per-row sources are cited so each line can be checked against the reading.
pub const MAIN_MENU: [MenuRow; MAIN_MENU_ROWS] = [
    // S03 §1 transport block (ids 40044-40048, 40154, 40195, 40197).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Playback,
        label: "Playback",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Playback),
    }),
    MenuRow::Separator,
    // S03 §1 file block (ids 40029, 40185, 40187, 40193, 40194, 40188).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Open,
        label: "Open",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Open),
    }),
    // S03 §1 bookmark block (ids 40320, 40321).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Bookmarks,
        label: "Bookmarks",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Bookmarks),
    }),
    MenuRow::Separator,
    // S03 §1 window/mode/dialog block (ids 40012, 40019, 40063, 40064,
    // 40157, 40186, 40224, 40258, 40291, 40334, 40041).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Options,
        label: "Options",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Options),
    }),
    // S03 §1 visualization block (ids 40191, 40192, 40221, 40317).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Visualization,
        label: "Visualization",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Visualization),
    }),
    // S03 §1 skin block (ids 40219, 40291).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Skins,
        label: "Skins",
        shortcut: None,
        check: MenuCheck::None,
        action: MenuAction::Submenu(SubmenuId::Skins),
    }),
    MenuRow::Separator,
    // Task card: checkable window rows (Main Window: S03 §1 id 40258;
    // Playlist Editor: S03 §1 id 40040; Equalizer: S03 §1 id 40036).
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::MainWindow,
        label: "Main Window",
        shortcut: None,
        check: MenuCheck::MainWindow,
        action: MenuAction::Shortcut(ShortcutAction::ShowMainWindow),
    }),
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::PlaylistEditor,
        label: "Playlist Editor",
        shortcut: None,
        check: MenuCheck::PlaylistEditor,
        action: MenuAction::Shortcut(ShortcutAction::TogglePlaylistEditor),
    }),
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::Equalizer,
        label: "Equalizer",
        shortcut: None,
        check: MenuCheck::Equalizer,
        action: MenuAction::Shortcut(ShortcutAction::ToggleEqualizer),
    }),
    MenuRow::Separator,
    // Task card: the time-display pair (S03 §1 id 40242, RFC 0005 §9
    // UI-local). Both rows flip the same flag; the check moves between them.
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::TimeElapsed,
        label: "Time elapsed",
        shortcut: None,
        check: MenuCheck::TimeElapsed,
        action: MenuAction::Shortcut(ShortcutAction::ToggleTimeRemaining),
    }),
    MenuRow::Item(MainMenuItem {
        id: MainMenuId::TimeRemaining,
        label: "Time remaining",
        shortcut: None,
        check: MenuCheck::TimeRemaining,
        action: MenuAction::Shortcut(ShortcutAction::ToggleTimeRemaining),
    }),
];

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

const OPEN_ENTRIES: [SubmenuItem; 6] = [
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

/// The entries of a submenu, top to bottom (contents S03-backed; the grouping
/// itself is provisional pending S04).
pub fn submenu_entries(id: SubmenuId) -> &'static [SubmenuItem] {
    match id {
        SubmenuId::Playback => &PLAYBACK_ENTRIES,
        SubmenuId::Open => &OPEN_ENTRIES,
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
// Geometry
// ---------------------------------------------------------------------------

/// The `(top, bottom)` pixel band of every top-level row under the
/// provisional 47/13 split (pending S04).
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
/// Our own drawing: two 3-px strokes forming a tick. It stays inside
/// [`CHECK_CELL_X`]; the task card states each measured check mark is a
/// 40-px difference confined to that cell, which this renderer mirrors only
/// in placement, not in pixels.
fn draw_check(canvas: &mut Image, band_top: u32) {
    let mid = band_top as i32 + (MAIN_MENU_ITEM_HEIGHT_PX / 2) as i32;
    // Short arm: (19, mid+1) to (23, mid+5).
    canvas.fill_rect(Rect::new(19, mid - 1, 3, 5), MENU_INK);
    canvas.fill_rect(Rect::new(21, mid + 1, 3, 5), MENU_INK);
    // Long arm: (23, mid+5) up to (29, mid-7).
    canvas.fill_rect(Rect::new(23, mid - 7, 3, 13), MENU_INK);
    canvas.fill_rect(Rect::new(25, mid - 7, 3, 10), MENU_INK);
}

/// Draws a right-pointing submenu arrow (our own drawing).
fn draw_arrow(canvas: &mut Image, band_top: u32) {
    let mid = band_top as i32 + (MAIN_MENU_ITEM_HEIGHT_PX / 2) as i32;
    for (i, width) in [9, 9, 7, 7, 5, 5, 3, 3].iter().enumerate() {
        canvas.fill_rect(
            Rect::new(ARROW_X, mid - 8 + i as i32 * 2, *width, 2),
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
            let x = MAIN_MENU_CLIENT_SIZE.0 as i32 - HINT_MARGIN_RIGHT as i32 - width as i32;
            font.render_text(canvas, x, baseline, &hint.to_ascii_lowercase());
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
                let mid = (top + bottom) / 2;
                canvas.fill_rect(Rect::new(4, mid as i32, width - 8, 1), SEPARATOR_DARK);
                canvas.fill_rect(Rect::new(4, mid as i32 + 1, width - 8, 1), SEPARATOR_LIGHT);
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
/// the longest label plus its hint (our layout helper — submenu sizes are
/// unmeasured, pending S04).
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
            content + HINT_MARGIN_RIGHT + 24
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
    fn the_table_has_fifteen_rows_with_four_separators() {
        assert_eq!(MAIN_MENU.len(), MAIN_MENU_ROWS);
        assert_eq!(MAIN_MENU.len(), 15);
        let separators = MAIN_MENU
            .iter()
            .filter(|row| matches!(row, MenuRow::Separator))
            .count();
        assert_eq!(separators, MAIN_MENU_SEPARATORS);
        assert_eq!(separators, 4);
        assert_eq!(item_rows().len(), 11);
    }

    #[test]
    fn separators_sit_between_the_groups() {
        let at: Vec<usize> = MAIN_MENU
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                MenuRow::Separator => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(at, [1, 4, 8, 12]);
    }

    #[test]
    fn six_rows_carry_submenus_and_each_has_entries() {
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
                (MainMenuId::Playback, SubmenuId::Playback),
                (MainMenuId::Open, SubmenuId::Open),
                (MainMenuId::Bookmarks, SubmenuId::Bookmarks),
                (MainMenuId::Options, SubmenuId::Options),
                (MainMenuId::Visualization, SubmenuId::Visualization),
                (MainMenuId::Skins, SubmenuId::Skins),
            ]
        );
        for (_, id) in &parents {
            assert!(!submenu_entries(*id).is_empty(), "{id:?} has entries");
        }
    }

    #[test]
    fn the_checkable_set_is_the_windows_plus_the_time_pair_plus_modes() {
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
                (MainMenuId::TimeElapsed, MenuCheck::TimeElapsed),
                (MainMenuId::TimeRemaining, MenuCheck::TimeRemaining),
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
    fn every_menu_action_runs_the_key_dispatch() {
        // No menu row may reference an action the dispatcher drops: leaves
        // must yield commands (or the handled time flip), never `Unhandled`.
        let mut machine = MainWindow::default();
        let mut leaves = Vec::new();
        for item in item_rows() {
            match item.action {
                MenuAction::Shortcut(action) => leaves.push((item.label, action)),
                MenuAction::Submenu(_) => {}
            }
        }
        for entry in [
            &PLAYBACK_ENTRIES[..],
            &OPEN_ENTRIES[..],
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
    fn the_time_pair_shares_the_ui_local_flip() {
        let elapsed = top_level(MainMenuId::TimeElapsed).unwrap();
        let remaining = top_level(MainMenuId::TimeRemaining).unwrap();
        assert_eq!(elapsed.action, remaining.action);
        assert_eq!(
            elapsed.action,
            MenuAction::Shortcut(ShortcutAction::ToggleTimeRemaining)
        );
        // The check moves between the two rows when the flag flips.
        let flags = MenuFlags::default();
        let mut state = MainWindowState::default();
        assert!(check_state(elapsed.check, &state, &flags));
        assert!(!check_state(remaining.check, &state, &flags));
        state.time_remaining = true;
        assert!(!check_state(elapsed.check, &state, &flags));
        assert!(check_state(remaining.check, &state, &flags));
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
        assert_eq!(row_at_y(46), Some(0));
        assert_eq!(row_at_y(47), Some(1));
        assert_eq!(row_at_y(568), Some(14));
        assert_eq!(row_at_y(569), None);
        // Every band maps back to its own index.
        for (index, (top, bottom)) in row_bands().iter().enumerate() {
            assert_eq!(row_at_y(*top), Some(index));
            assert_eq!(row_at_y(bottom - 1), Some(index));
        }
    }
}
