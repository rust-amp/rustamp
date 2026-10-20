// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The native preferences dialog.
//!
//! The reference's preferences window is an ordinary Windows dialog: a tree of
//! pages on the left with a `Close` button under it, and the selected page's
//! group boxes, check boxes and lists on the right, all in the system font.
//! The player used to paint a frame of its own with the skin's bitmap font and
//! no controls at all, which is why it looked wrong and did nothing.
//!
//! This module is that dialog, in the layer that owns the Win32 bindings.
//!
//! # The headless seam
//!
//! The window needs a desktop. What does not is the whole *model*: the page
//! tree ([`TREE`]), what each page contains ([`page_items`]), which control is
//! bound to which setting ([`Bind`]) and what a click turns into
//! ([`PrefsAction`]). These are plain data and are unit-tested below.
//!
//! # Honesty
//!
//! A control exists only where the player really has the setting behind it.
//! Pages without any setting say so in words instead of showing dead controls.
//!
//! # The audio settings
//!
//! Two settings are not a yes/no but a list of values: the ReplayGain mode
//! (off, track, album) and the output mode (shared, exclusive). They are
//! drop-down lists ([`Item::Combo`]) bound to one of the two
//! ([`ComboBind`]), and the list's order ([`REPLAYGAIN_CHOICES`],
//! [`OUTPUT_MODE_CHOICES`]) is the single place where the dialog, the
//! configuration and the player agree on what a row means.
//!
//! The reference's preferences pages are unmeasured (S04 is open), so the page
//! that carries them and the wording of their labels are own choices.
//!
//! # The plug-in list
//!
//! Five of the pages list plug-ins: one page per kind. The rows of such a
//! list belong to the page they are shown on - the input page's rows say
//! nothing about the output page's rows - so the list remembers the page it
//! shows. A page switch therefore refills the list, even when the two pages
//! have the same rows: the player says the same thing on every plug-in page
//! when it has nothing to list.
//!
//! The player's id behind a row is part of that state as well, so that the
//! "switch on / off" button always names the plug-in of the row that is on
//! screen.
//!
//! The selection is the user's row, and it is part of the state as well: the
//! "switch on / off" button acts on the row that is on screen. A plug-in
//! that is not among the rows any more is gone: the first row takes its
//! place, or nothing when the list is empty.
//!
//! # The skin list
//!
//! The Skins page lists the skins the player can load. The rows come from
//! the player ([`PrefsSettings::skins`]) and they can change while the
//! dialog is open: the skin in use is one of the rows, so loading another
//! skin changes the list. The list therefore compares the rows it shows
//! with the rows the player pushes and refills itself only when they
//! differ - a push that changes nothing does not flicker the list.
//!
//! The selection is the user's row. The player pushes the skin in use with
//! every frame, and such a push must not take back a row the user picked
//! before the player got the pick: the pick wins until the player names
//! that skin or names another one. A skin that is not among the rows any
//! more is gone: the first row takes its place, or nothing when the list is
//! empty.
//!
//! # The three lists
//!
//! The plug-in list, the skin list and the media library's root list are one
//! state (`PageLists`): a page switch builds new controls, so the lists
//! start empty - the rows of the page the user came from say nothing about
//! the new one - and the player pushes into the lists the page has.
//!
//! # The library page
//!
//! One page is not the reference's: `Library`. The media library is this
//! player's own feature, and the folders it indexes are a setting the user
//! has to be able to change. The page lists the roots
//! ([`PrefsSettings::library_roots`]) and offers two actions: add a folder
//! ([`PrefsAction::AddLibraryRoot`]) and remove the selected root
//! ([`PrefsAction::RemoveLibraryRoot`]).
//!
//! The roots are **not** a setting of the configuration file: they live in
//! the media library's own database, and the player reads them from there.
//! The dialog only shows them and says what the user did; the player turns
//! the action into a change of the index.
//!
//! The list of the roots is a state like the other two: it compares the rows
//! it shows with the rows the player pushes, so a push that changes nothing
//! does not flicker it, and the selection is the user's row. A root that is
//! not among the rows any more is gone: the first row takes its place, or
//! nothing when the list is empty.
//!
//! # The row of a root
//!
//! A row is the root's path and the time of its last **finished** scan
//! ([`LibraryRootRow`]): `C:\Music — last scanned 2026-10-20 03:11 UTC`, or
//! `C:\Music — never scanned` for a root that was not scanned yet.
//!
//! The time is **UTC** and it names the **minute**, not the second: the row
//! says when the root was looked at, and a minute is what that means for the
//! user. UTC is the choice because it does not depend on the time zone of
//! the machine that shows the row.
//!
//! The row is one line, and the list scrolls sideways, so a long path stays
//! readable: the list box carries a horizontal scroll bar, and its scroll
//! range reaches the end of the longest row.

use rustamp_core::config::{OutputMode, ReplayGainMode};

use crate::utc::utc_calendar;

/// One node of the page tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefsNode {
    /// `Setup`.
    Setup,
    /// `Setup > File types`.
    FileTypes,
    /// `Options`.
    Options,
    /// `Options > Display`.
    Display,
    /// `Options > Visualization`.
    OptionsVisualization,
    /// `Options > Shuffle`.
    Shuffle,
    /// `Plug-ins`.
    PlugIns,
    /// `Plug-ins > Input`.
    Input,
    /// `Plug-ins > Output`.
    Output,
    /// `Plug-ins > Visualization`.
    PlugInVisualization,
    /// `Plug-ins > DSP/Effect`.
    DspEffect,
    /// `Plug-ins > General Purpose`.
    GeneralPurpose,
    /// `Skins`.
    Skins,
    /// `Library`.
    ///
    /// **Own choice.** The reference's tree has no such page (S04 is open);
    /// the media library is this player's own feature, and its roots are a
    /// setting the user has to be able to change.
    Library,
    /// `Bookmarks`.
    Bookmarks,
}

/// One row of the page tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeEntry {
    /// The page this row selects.
    pub node: PrefsNode,
    /// The row text.
    pub label: &'static str,
    /// Nesting depth: `0` is a root row, `1` a child of the root row above.
    pub depth: u8,
}

const fn entry(node: PrefsNode, label: &'static str, depth: u8) -> TreeEntry {
    TreeEntry { node, label, depth }
}

/// The page tree, top to bottom (parents come before their children).
pub const TREE: &[TreeEntry] = &[
    entry(PrefsNode::Setup, "Setup", 0),
    entry(PrefsNode::FileTypes, "File types", 1),
    entry(PrefsNode::Options, "Options", 0),
    entry(PrefsNode::Display, "Display", 1),
    entry(PrefsNode::OptionsVisualization, "Visualization", 1),
    entry(PrefsNode::Shuffle, "Shuffle", 1),
    entry(PrefsNode::PlugIns, "Plug-ins", 0),
    entry(PrefsNode::Input, "Input", 1),
    entry(PrefsNode::Output, "Output", 1),
    entry(PrefsNode::PlugInVisualization, "Visualization", 1),
    entry(PrefsNode::DspEffect, "DSP/Effect", 1),
    entry(PrefsNode::GeneralPurpose, "General Purpose", 1),
    entry(PrefsNode::Skins, "Skins", 0),
    entry(PrefsNode::Library, "Library", 0),
    entry(PrefsNode::Bookmarks, "Bookmarks", 0),
];

/// A rectangle in unscaled (96 DPI) pixels, relative to the page area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left.
    pub x: i32,
    /// Top.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

const fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

/// A setting a check box is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bind {
    /// Keep the main window above other windows.
    AlwaysOnTop,
    /// Drag the windows from anywhere.
    EasyMove,
    /// Draw the windows at double size.
    DoubleSize,
    /// Play in random order.
    Shuffle,
    /// Repeat the playlist.
    Repeat,
}

/// A setting a drop-down list is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComboBind {
    /// The ReplayGain mode: off, track or album.
    ReplayGain,
    /// The output mode: shared or exclusive.
    OutputMode,
}

/// The ReplayGain modes the list shows, in order.
///
/// The order is the list's row order, so it is also the order the rows are
/// numbered by. Every mode of the configuration is in it.
pub const REPLAYGAIN_CHOICES: [ReplayGainMode; 3] = [
    ReplayGainMode::Off,
    ReplayGainMode::Track,
    ReplayGainMode::Album,
];

/// The output modes the list shows, in order.
///
/// The order is the list's row order, so it is also the order the rows are
/// numbered by. Every mode of the configuration is in it.
pub const OUTPUT_MODE_CHOICES: [OutputMode; 2] = [OutputMode::Shared, OutputMode::Exclusive];

/// The text of the list row that shows `mode`.
pub fn replaygain_label(mode: ReplayGainMode) -> &'static str {
    match mode {
        ReplayGainMode::Off => "Off",
        ReplayGainMode::Track => "Track",
        ReplayGainMode::Album => "Album",
    }
}

/// The text of the list row that shows `mode`.
pub fn output_mode_label(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::Shared => "Shared",
        OutputMode::Exclusive => "Exclusive",
    }
}

/// The row that shows `mode`.
///
/// Every mode is in [`REPLAYGAIN_CHOICES`] (a test pins that), so the fallback
/// is unreachable and only keeps the function total.
pub fn replaygain_row(mode: ReplayGainMode) -> usize {
    REPLAYGAIN_CHOICES
        .iter()
        .position(|m| *m == mode)
        .unwrap_or(0)
}

/// The mode the row `row` shows, or `None` for a row that does not exist.
pub fn replaygain_at(row: usize) -> Option<ReplayGainMode> {
    REPLAYGAIN_CHOICES.get(row).copied()
}

/// The row that shows `mode`.
///
/// Every mode is in [`OUTPUT_MODE_CHOICES`] (a test pins that), so the
/// fallback is unreachable and only keeps the function total.
pub fn output_mode_row(mode: OutputMode) -> usize {
    OUTPUT_MODE_CHOICES
        .iter()
        .position(|m| *m == mode)
        .unwrap_or(0)
}

/// The mode the row `row` shows, or `None` for a row that does not exist.
pub fn output_mode_at(row: usize) -> Option<OutputMode> {
    OUTPUT_MODE_CHOICES.get(row).copied()
}

impl ComboBind {
    /// The texts of the rows the list shows, in order.
    pub fn labels(self) -> Vec<&'static str> {
        match self {
            ComboBind::ReplayGain => REPLAYGAIN_CHOICES
                .iter()
                .copied()
                .map(replaygain_label)
                .collect(),
            ComboBind::OutputMode => OUTPUT_MODE_CHOICES
                .iter()
                .copied()
                .map(output_mode_label)
                .collect(),
        }
    }

    /// The action the row `row` stands for, or `None` for a row that does not
    /// exist.
    pub fn action(self, row: usize) -> Option<PrefsAction> {
        match self {
            ComboBind::ReplayGain => replaygain_at(row).map(PrefsAction::SetReplayGain),
            ComboBind::OutputMode => output_mode_at(row).map(PrefsAction::SetOutputMode),
        }
    }
}

/// One element of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A group box with a caption.
    Group {
        /// Caption.
        text: &'static str,
        /// Placement.
        rect: Rect,
    },
    /// A line of static text.
    Label {
        /// Text.
        text: &'static str,
        /// Placement.
        rect: Rect,
    },
    /// A check box bound to a setting.
    Check {
        /// Text.
        text: &'static str,
        /// Placement.
        rect: Rect,
        /// The setting it shows and changes.
        bind: Bind,
    },
    /// A drop-down list with a label in front of it.
    Combo {
        /// The label's text.
        text: &'static str,
        /// Where the label sits.
        label: Rect,
        /// Where the list sits. Its height is the height of the **drop-down
        /// list**, not of the closed box: the closed box is drawn at the
        /// standard height at the top of the rectangle, so the rectangle must
        /// not reach over the group box's frame.
        list: Rect,
        /// The setting it shows and changes.
        bind: ComboBind,
    },
    /// The list of loadable skins; choosing a row loads it.
    SkinList {
        /// Placement.
        rect: Rect,
    },
    /// The plug-ins of the page's kind, one row each ([`PluginRow`]).
    PluginList {
        /// Placement.
        rect: Rect,
    },
    /// Switches the selected plug-in row on or off ([`PrefsAction::TogglePlugin`]).
    PluginToggle {
        /// Placement.
        rect: Rect,
    },
    /// Reads the plug-in folder again ([`PrefsAction::RescanPlugins`]).
    PluginRescan {
        /// Placement.
        rect: Rect,
    },
    /// The media library's roots, one row each.
    LibraryRoots {
        /// Placement.
        rect: Rect,
    },
    /// Asks for a folder to add to the media library
    /// ([`PrefsAction::AddLibraryRoot`]).
    LibraryAdd {
        /// Placement.
        rect: Rect,
    },
    /// Removes the selected root ([`PrefsAction::RemoveLibraryRoot`]).
    LibraryRemove {
        /// Placement.
        rect: Rect,
    },
}

/// Width and height of the page area.
pub const PAGE_SIZE: (i32, i32) = (262, 284);

fn note_page(group: &'static str, text: &'static str) -> Vec<Item> {
    vec![
        Item::Group {
            text: group,
            rect: rect(0, 0, PAGE_SIZE.0, 80),
        },
        Item::Label {
            text,
            rect: rect(12, 22, PAGE_SIZE.0 - 24, 48),
        },
    ]
}

/// The layout every plug-in page shares: the caption, a hint, the list and the
/// two buttons under it.
fn plugin_page(caption: &'static str, hint: &'static str) -> Vec<Item> {
    let w = PAGE_SIZE.0;
    vec![
        Item::Group {
            text: caption,
            rect: rect(0, 0, w, PAGE_SIZE.1),
        },
        Item::Label {
            text: hint,
            rect: rect(12, 20, w - 24, 56),
        },
        Item::PluginList {
            rect: rect(10, 80, w - 20, 158),
        },
        Item::PluginToggle {
            rect: rect(10, 246, 150, 26),
        },
        Item::PluginRescan {
            rect: rect(168, 246, w - 178, 26),
        },
    ]
}

/// Hint under every plug-in page's caption. The approval gate and the fact
/// that nothing is loaded at startup yet are stated, not implied.
const PLUGIN_HINT: &str = "Only approved plug-ins can be switched on. \
    The plug-ins you switch on are loaded when the plugin host starts.";

/// What `node`'s page contains.
pub fn page_items(node: PrefsNode) -> Vec<Item> {
    let w = PAGE_SIZE.0;
    match node {
        // The audio settings live on the Setup page: it is the page for the
        // device, and both settings are about how the player sounds. The
        // group box is tall enough for the drop-down lists, so that a list
        // never reaches over the group's frame.
        PrefsNode::Setup => vec![
            Item::Group {
                text: "Audio",
                rect: rect(0, 0, w, 160),
            },
            Item::Combo {
                text: "ReplayGain:",
                label: rect(12, 22, 100, 20),
                list: rect(118, 20, 132, 60),
                bind: ComboBind::ReplayGain,
            },
            Item::Combo {
                text: "Output mode:",
                label: rect(12, 100, 100, 20),
                list: rect(118, 98, 132, 60),
                bind: ComboBind::OutputMode,
            },
            Item::Group {
                text: "General",
                rect: rect(0, 168, w, 72),
            },
            Item::Label {
                text: "Language: English.\nSettings are saved to the player's configuration file when it closes.",
                rect: rect(12, 188, w - 24, 48),
            },
        ],
        PrefsNode::FileTypes => note_page(
            "File types",
            "Associating file types with Rustamp is not available yet.",
        ),
        PrefsNode::Options => vec![
            Item::Group {
                text: "Windows",
                rect: rect(0, 0, w, 76),
            },
            Item::Check {
                text: "Always on top",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::AlwaysOnTop,
            },
            Item::Check {
                text: "Easy move (drag windows from anywhere)",
                rect: rect(12, 46, w - 24, 20),
                bind: Bind::EasyMove,
            },
        ],
        PrefsNode::Display => vec![
            Item::Group {
                text: "Display",
                rect: rect(0, 0, w, 52),
            },
            Item::Check {
                text: "Double size",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::DoubleSize,
            },
        ],
        PrefsNode::OptionsVisualization => note_page(
            "Visualization",
            "The visualization options are not configurable yet.",
        ),
        PrefsNode::Shuffle => vec![
            Item::Group {
                text: "Playback order",
                rect: rect(0, 0, w, 76),
            },
            Item::Check {
                text: "Shuffle",
                rect: rect(12, 22, w - 24, 20),
                bind: Bind::Shuffle,
            },
            Item::Check {
                text: "Repeat",
                rect: rect(12, 46, w - 24, 20),
                bind: Bind::Repeat,
            },
        ],
        PrefsNode::PlugIns => {
            note_page("Plug-ins", "Choose a plug-in type in the list on the left.")
        }
        PrefsNode::Input => plugin_page("Input plug-ins", PLUGIN_HINT),
        PrefsNode::Output => plugin_page("Output plug-ins", PLUGIN_HINT),
        PrefsNode::PlugInVisualization => plugin_page("Visualization plug-ins", PLUGIN_HINT),
        PrefsNode::DspEffect => plugin_page("DSP/Effect plug-ins", PLUGIN_HINT),
        PrefsNode::GeneralPurpose => plugin_page("General purpose plug-ins", PLUGIN_HINT),
        PrefsNode::Skins => vec![
            Item::Group {
                text: "Select skin",
                rect: rect(0, 0, w, PAGE_SIZE.1),
            },
            Item::SkinList {
                rect: rect(10, 20, w - 20, PAGE_SIZE.1 - 30),
            },
        ],
        PrefsNode::Library => vec![
            Item::Group {
                text: "Media library folders",
                rect: rect(0, 0, w, PAGE_SIZE.1),
            },
            Item::Label {
                text: "The media library indexes the tracks in these folders.",
                rect: rect(12, 20, w - 24, 36),
            },
            Item::LibraryRoots {
                rect: rect(10, 60, w - 20, 150),
            },
            Item::LibraryAdd {
                rect: rect(10, 218, 120, 26),
            },
            Item::LibraryRemove {
                rect: rect(138, 218, 114, 26),
            },
        ],
        PrefsNode::Bookmarks => note_page("Bookmarks", "Bookmarks are not available yet."),
    }
}

/// One row of a plug-in page's list, prepared by the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    /// The page whose list shows this row.
    pub node: PrefsNode,
    /// The player's id for the plug-in, echoed back in
    /// [`PrefsAction::TogglePlugin`]. `None` for an information row (such as
    /// "no plug-ins found"), which the toggle button ignores.
    pub id: Option<usize>,
    /// The row text.
    pub text: String,
}

/// One row of the media library's root list.
///
/// The row is a root of the media library's index and the time of its last
/// finished scan. The time is a unix second; `None` means the root was
/// never scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRootRow {
    /// The root's path, as it was added.
    pub path: String,
    /// Unix seconds of the root's last **finished** scan.
    ///
    /// `None` when the root was never scanned. A scan that was cancelled or
    /// that failed does not count: it did not finish, so it must not claim
    /// that the root was scanned.
    pub last_scanned_at: Option<i64>,
}

/// The text of a media library root's row.
///
/// The path, and the time of the root's last finished scan as
/// `YYYY-MM-DD HH:MM UTC`. A root that was never scanned says so.
pub fn library_root_text(path: &str, last_scanned_at: Option<i64>) -> String {
    match last_scanned_at {
        Some(at) => {
            let when = utc_calendar(at);
            format!(
                "{path} — last scanned {:04}-{:02}-{:02} {:02}:{:02} UTC",
                when.year, when.month, when.day, when.hour, when.minute
            )
        }
        None => format!("{path} — never scanned"),
    }
}

/// The settings the dialog shows, pushed in by the player.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefsSettings {
    /// Always on top.
    pub always_on_top: bool,
    /// Easy move.
    pub easy_move: bool,
    /// Double size.
    pub double_size: bool,
    /// Shuffle.
    pub shuffle: bool,
    /// Repeat.
    pub repeat: bool,
    /// The ReplayGain mode.
    pub replaygain: ReplayGainMode,
    /// The output mode.
    pub output_mode: OutputMode,
    /// The skins that can be loaded, one row each.
    ///
    /// The rows can change while the dialog is open (the skin in use is one
    /// of them), so the dialog compares them with the rows it shows and
    /// refills the list only when they differ.
    pub skins: Vec<String>,
    /// The row of the skin in use, counted in [`skins`](Self::skins).
    ///
    /// `None` when the player has no skin in use.
    pub skin_selected: Option<usize>,
    /// The rows of every plug-in page's list.
    pub plugins: Vec<PluginRow>,
    /// The media library's roots, one row each.
    ///
    /// The roots are not a setting of the configuration file: they live in
    /// the media library's own database, and the player reads them from
    /// there.
    pub library_roots: Vec<LibraryRootRow>,
}

impl PrefsSettings {
    /// The rows `node`'s list shows, in order.
    pub fn plugin_rows(&self, node: PrefsNode) -> impl Iterator<Item = &PluginRow> {
        self.plugins.iter().filter(move |row| row.node == node)
    }

    /// The current value of `bind`.
    pub fn get(&self, bind: Bind) -> bool {
        match bind {
            Bind::AlwaysOnTop => self.always_on_top,
            Bind::EasyMove => self.easy_move,
            Bind::DoubleSize => self.double_size,
            Bind::Shuffle => self.shuffle,
            Bind::Repeat => self.repeat,
        }
    }

    /// The row the list of `bind` shows.
    pub fn combo_row(&self, bind: ComboBind) -> usize {
        match bind {
            ComboBind::ReplayGain => replaygain_row(self.replaygain),
            ComboBind::OutputMode => output_mode_row(self.output_mode),
        }
    }

    /// The media library's root in the row `row` of the library's list.
    ///
    /// `None` for a row that does not exist: the row is the one the user
    /// picked, and a stale one must not remove another root.
    pub fn library_root(&self, row: usize) -> Option<&str> {
        self.library_roots.get(row).map(|root| root.path.as_str())
    }
}

/// What the plug-in list of a page shows, and which row is selected.
///
/// The rows of the plug-in list belong to the page they are shown on: the
/// input page's rows say nothing about the output page's rows. The page is
/// therefore part of the state, so that a page switch refills the list even
/// when the two pages have the same rows - which they do when the player has
/// nothing to list and says so with the same text on every page.
///
/// The state also carries the player's id behind each row, so that the id
/// always belongs to the row that is on screen.
///
/// The selection is the user's row. A plug-in that is not among the rows any
/// more is gone: the first row takes its place, or nothing when the list is
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PluginListState {
    /// The page the rows belong to; `None` when no page was shown yet.
    node: Option<PrefsNode>,
    /// The player's id behind each row.
    ids: Vec<Option<usize>>,
    /// The text of each row.
    texts: Vec<String>,
    /// The row the user picked.
    selected: Option<usize>,
}

/// What the plug-in list does with a push from the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PluginListUpdate {
    /// The rows changed: the list must be refilled before the selection is
    /// set.
    pub(crate) refill: bool,
    /// The row to select; `None` when no row is selected.
    pub(crate) selected: Option<usize>,
}

impl PluginListState {
    /// The rows `node`'s list shows, with the player's id behind each.
    pub(crate) fn of(settings: &PrefsSettings, node: PrefsNode) -> Self {
        let (ids, texts): (Vec<Option<usize>>, Vec<String>) = settings
            .plugin_rows(node)
            .map(|row| (row.id, row.text.clone()))
            .unzip();
        Self {
            node: Some(node),
            ids,
            texts,
            selected: None,
        }
    }

    /// The list of the page `node` that was just built: it shows nothing yet.
    ///
    /// A page switch builds new controls, so the list starts empty: the rows
    /// of the page the user came from say nothing about the new one.
    pub(crate) fn shown(node: PrefsNode) -> Self {
        Self {
            node: Some(node),
            ..Self::default()
        }
    }

    /// The text of each row, in order.
    pub(crate) fn texts(&self) -> &[String] {
        &self.texts
    }

    /// The player's id behind the row `row`, if that row has one.
    pub(crate) fn id(&self, row: usize) -> Option<usize> {
        self.ids.get(row).copied().flatten()
    }

    /// Whether a list that shows these rows already shows `state`.
    ///
    /// The rows of another page are never the rows of this page, even when
    /// their texts are the same: the list is rebuilt for every page.
    pub(crate) fn shows(&self, state: &PluginListState) -> bool {
        self.node == state.node && self.ids == state.ids && self.texts == state.texts
    }

    /// The user picked the row `row` of the list.
    ///
    /// A row that does not exist is not a pick: the selection stays what it
    /// was.
    pub(crate) fn pick(&mut self, row: usize) {
        if row < self.texts.len() {
            self.selected = Some(row);
        }
    }

    /// What the list does with the player's push.
    ///
    /// The rows are refilled only when they differ from the rows the list
    /// shows. The selection is the row the user picked, as long as that row
    /// is still there: a plug-in that is gone leaves the first row in its
    /// place, or nothing when the list is empty.
    pub(crate) fn apply(&mut self, settings: &PrefsSettings, node: PrefsNode) -> PluginListUpdate {
        let wanted = PluginListState::of(settings, node);
        let refill = !self.shows(&wanted);
        let selected = match self.selected {
            Some(row) if row < wanted.texts.len() => Some(row),
            // The row is gone: the first row takes its place, or nothing when
            // the list is empty.
            Some(_) if wanted.texts.is_empty() => None,
            Some(_) => Some(0),
            None => None,
        };
        self.node = wanted.node;
        self.ids = wanted.ids;
        self.texts = wanted.texts;
        self.selected = selected;
        PluginListUpdate { refill, selected }
    }
}

/// What the skin list shows, and what the user picked in it.
///
/// The rows of the list come from the player ([`PrefsSettings::skins`]) and
/// they can change while the dialog is open: a skin that appears in the
/// folder, or the skin that is in use, changes them. The state therefore
/// remembers the rows it shows and compares them with the rows the player
/// pushes: only a change refills the list, so the per-tick push does not
/// flicker it.
///
/// The rows belong to the page they are shown on, exactly like the plug-in
/// list's rows: the list of one page never passes for the list of another,
/// so a page switch refills it.
///
/// The selection is part of the state as well, because the player pushes the
/// skin in use **every frame** and such a push must not take back the row
/// the user picked before the player got the pick:
///
/// * the user's pick wins until the player names that skin or names another
///   one;
/// * a skin that is not among the rows any more is gone: the first row
///   takes its place, or nothing when there is no row at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SkinListState {
    /// The page the rows belong to; `None` when no page was shown yet.
    node: Option<PrefsNode>,
    /// The row texts the list shows.
    skins: Vec<String>,
    /// The skin the user picked, waiting for the player to confirm it.
    picked: Option<String>,
    /// The skin the player last reported as the one in use.
    player: Option<String>,
}

/// What the skin list does with a push from the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkinListUpdate {
    /// The rows changed: the list must be refilled before the selection is
    /// set.
    pub(crate) refill: bool,
    /// The row to select; `None` when no row is selected.
    pub(crate) selected: Option<usize>,
}

impl SkinListState {
    /// The state of the Skins page's list: the rows `settings` pushes and
    /// the skin in use among them.
    pub(crate) fn of(settings: &PrefsSettings) -> Self {
        let skins = settings.skins.clone();
        let player = settings
            .skin_selected
            .and_then(|row| skins.get(row))
            .cloned();
        Self {
            node: Some(PrefsNode::Skins),
            skins,
            picked: None,
            player,
        }
    }

    /// The list of the Skins page that was just built: it shows nothing yet.
    ///
    /// A page switch builds new controls, so the list starts empty: the rows
    /// of the page the user came from say nothing about the new one.
    pub(crate) fn shown() -> Self {
        Self {
            node: Some(PrefsNode::Skins),
            ..Self::default()
        }
    }

    /// The text of each row, in order.
    pub(crate) fn texts(&self) -> &[String] {
        &self.skins
    }

    /// Whether a list that shows these rows already shows `state`.
    ///
    /// The rows of another page are never the rows of this one, so the page
    /// is part of the comparison: a list that showed nothing yet is never
    /// the list of the Skins page.
    pub(crate) fn shows(&self, state: &SkinListState) -> bool {
        self.node == state.node && self.skins == state.skins
    }

    /// The user picked the row `row` of the list.
    ///
    /// The pick is the skin's **name**, not its row: the rows can change
    /// while the pick waits for the player, and the skin the user picked is
    /// then found in the new rows.
    pub(crate) fn pick(&mut self, row: usize) {
        if let Some(skin) = self.skins.get(row) {
            self.picked = Some(skin.clone());
        }
    }

    /// What the list does with the player's push.
    ///
    /// The rows are refilled only when they differ from the rows the list
    /// shows. The selection is the user's pick while the player has not
    /// named it, else the skin the player says is in use. A skin that is not
    /// among the rows any more is gone: the first row takes its place, or
    /// nothing when there is no row at all.
    pub(crate) fn apply(&mut self, settings: &PrefsSettings) -> SkinListUpdate {
        let wanted = SkinListState::of(settings);
        let refill = !self.shows(&wanted);
        let player = wanted.player.clone();
        let pick = self.picked.clone();

        // The user's pick wins while the player has not named it: a push
        // that still names the old skin must not take the pick back.
        let player_wins = match pick.as_deref() {
            Some(pick) if player.as_deref() == Some(pick) => {
                // The player took the pick: it is the skin in use now.
                self.picked = None;
                true
            }
            Some(_) if player.is_some() && player != self.player => {
                // The player changed the skin on its own: it wins over the
                // pick the player has not got yet.
                self.picked = None;
                true
            }
            Some(_) => false,
            // Without a pick the player decides.
            None => true,
        };
        let chosen = if player_wins { player.clone() } else { pick };
        self.player = player;

        // The row of the chosen skin. A skin that is not among the rows any
        // more is gone: the first row takes its place, or nothing when the
        // list is empty.
        let selected = match chosen {
            Some(skin) => match wanted.skins.iter().position(|row| *row == skin) {
                Some(row) => Some(row),
                None => {
                    // The skin is gone: the pick can never be confirmed.
                    self.picked = None;
                    if wanted.skins.is_empty() {
                        None
                    } else {
                        Some(0)
                    }
                }
            },
            None => None,
        };

        self.node = wanted.node;
        self.skins = wanted.skins;
        SkinListUpdate { refill, selected }
    }
}

/// What the media library's root list shows, and which row is selected.
///
/// The rows come from the player ([`PrefsSettings::library_roots`]) and they
/// can change while the dialog is open: a root that is added or removed
/// changes them. The state therefore remembers the rows it shows and
/// compares them with the rows the player pushes: only a change refills the
/// list, so the per-tick push does not flicker it.
///
/// The rows belong to the page they are shown on, exactly like the plug-in
/// list's rows: the list of one page never passes for the list of another,
/// so a page switch refills it.
///
/// The selection is the user's row. A root that is not among the rows any
/// more is gone: the first row takes its place, or nothing when the list is
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RootListState {
    /// The page the rows belong to; `None` when no page was shown yet.
    node: Option<PrefsNode>,
    /// The text of each row.
    texts: Vec<String>,
    /// The row the user picked.
    selected: Option<usize>,
}

/// What the media library's root list does with a push from the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RootListUpdate {
    /// The rows changed: the list must be refilled before the selection is
    /// set.
    pub(crate) refill: bool,
    /// The row to select; `None` when no row is selected.
    pub(crate) selected: Option<usize>,
}

impl RootListState {
    /// The state of the Library page's list: the rows `settings` pushes.
    pub(crate) fn of(settings: &PrefsSettings) -> Self {
        let texts = settings
            .library_roots
            .iter()
            .map(|root| library_root_text(&root.path, root.last_scanned_at))
            .collect();
        Self {
            node: Some(PrefsNode::Library),
            texts,
            selected: None,
        }
    }

    /// The list of the Library page that was just built: it shows nothing
    /// yet.
    ///
    /// A page switch builds new controls, so the list starts empty: the rows
    /// of the page the user came from say nothing about the new one.
    pub(crate) fn shown() -> Self {
        Self {
            node: Some(PrefsNode::Library),
            ..Self::default()
        }
    }

    /// The text of each row, in order.
    pub(crate) fn texts(&self) -> &[String] {
        &self.texts
    }

    /// Whether a list that shows these rows already shows `state`.
    ///
    /// The rows of another page are never the rows of this one, so the page
    /// is part of the comparison: a list that showed nothing yet is never
    /// the list of the Library page.
    pub(crate) fn shows(&self, state: &RootListState) -> bool {
        self.node == state.node && self.texts == state.texts
    }

    /// The user picked the row `row` of the list.
    ///
    /// A row that does not exist is not a pick: the selection stays what it
    /// was.
    pub(crate) fn pick(&mut self, row: usize) {
        if row < self.texts.len() {
            self.selected = Some(row);
        }
    }

    /// What the list does with the player's push.
    ///
    /// The rows are refilled only when they differ from the rows the list
    /// shows. The selection is the row the user picked, as long as that row
    /// is still there: a root that is gone leaves the first row in its
    /// place, or nothing when the list is empty.
    pub(crate) fn apply(&mut self, settings: &PrefsSettings) -> RootListUpdate {
        let wanted = RootListState::of(settings);
        let refill = !self.shows(&wanted);
        let selected = match self.selected {
            Some(row) if row < wanted.texts.len() => Some(row),
            // The row is gone: the first row takes its place, or nothing when
            // the list is empty.
            Some(_) if wanted.texts.is_empty() => None,
            Some(_) => Some(0),
            None => None,
        };
        self.node = wanted.node;
        self.texts = wanted.texts;
        self.selected = selected;
        RootListUpdate { refill, selected }
    }
}

/// The state of the three lists of the dialog.
///
/// A page switch builds new controls, so the lists start empty: the rows of
/// the page the user came from say nothing about the new one. The three
/// states therefore belong together - they are built and filled together,
/// and a page switch replaces all three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageLists {
    /// The plug-in list of the page.
    plugins: PluginListState,
    /// The skin list of the page.
    skins: SkinListState,
    /// The media library's root list of the page.
    roots: RootListState,
}

impl PageLists {
    /// The lists of the page `node` that was just built: they show nothing
    /// yet.
    pub(crate) fn shown(node: PrefsNode) -> Self {
        Self {
            plugins: PluginListState::shown(node),
            skins: SkinListState::shown(),
            roots: RootListState::shown(),
        }
    }

    /// The plug-in list of the page.
    pub(crate) fn plugins(&self) -> &PluginListState {
        &self.plugins
    }

    /// The skin list of the page.
    pub(crate) fn skins(&self) -> &SkinListState {
        &self.skins
    }

    /// The media library's root list of the page.
    pub(crate) fn roots(&self) -> &RootListState {
        &self.roots
    }

    /// The plug-in list of the page, for the user's pick.
    pub(crate) fn plugins_mut(&mut self) -> &mut PluginListState {
        &mut self.plugins
    }

    /// The skin list of the page, for the user's pick.
    pub(crate) fn skins_mut(&mut self) -> &mut SkinListState {
        &mut self.skins
    }

    /// The media library's root list of the page, for the user's pick.
    pub(crate) fn roots_mut(&mut self) -> &mut RootListState {
        &mut self.roots
    }
}

/// What the user did in the dialog; the player turns it into a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsAction {
    /// A check box was set to this value.
    Set(Bind, bool),
    /// The ReplayGain mode was set to this value.
    SetReplayGain(ReplayGainMode),
    /// The output mode was set to this value.
    SetOutputMode(OutputMode),
    /// A skin row was chosen.
    LoadSkin(usize),
    /// The toggle button was pressed with this plug-in selected
    /// ([`PluginRow::id`]).
    TogglePlugin(usize),
    /// The plug-in folder should be read again.
    RescanPlugins,
    /// A folder should be added to the media library.
    ///
    /// The player asks the user for the folder itself; the dialog only says
    /// that a folder was asked for.
    AddLibraryRoot,
    /// The media library's root in this row should be removed.
    ///
    /// The row is a row of [`PrefsSettings::library_roots`].
    RemoveLibraryRoot(usize),
}

/// Why the dialog could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    /// A Win32 call refused.
    #[error("preferences dialog: {0}")]
    Win32(String),
    /// There is no native dialog on this platform yet.
    #[error("a native preferences dialog is only implemented on Windows")]
    Unsupported,
}

#[cfg(windows)]
pub use win32::PrefsWindow;

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        COLOR_BTNFACE, CreateFontIndirectW, DeleteObject, GetDC, GetSysColorBrush,
        GetTextExtentPoint32W, HFONT, ReleaseDC,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Controls::{
        ICC_STANDARD_CLASSES, ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
        NMHDR, NMTREEVIEWW, TVE_EXPAND, TVGN_CARET, TVI_LAST, TVIF_PARAM, TVIF_TEXT,
        TVINSERTSTRUCTW, TVITEMW, TVM_EXPAND, TVM_INSERTITEMW, TVM_SELECTITEM, TVN_SELCHANGEDW,
        WC_TREEVIEWW,
    };
    use windows::Win32::UI::HiDpi::{
        AdjustWindowRectExForDpi, GetDpiForWindow, SystemParametersInfoForDpi,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BM_GETCHECK, BM_SETCHECK, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, HMENU, IDC_ARROW, IsWindow,
        LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LB_SETHORIZONTALEXTENT,
        LoadCursorW, NONCLIENTMETRICSW, RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_SHOW,
        SendMessageW, SetForegroundWindow, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND,
        WM_DESTROY, WM_NOTIFY, WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE,
        WS_HSCROLL, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::{
        Bind, ComboBind, Item, PageLists, PrefsAction, PrefsError, PrefsNode, PrefsSettings, Rect,
        TREE, page_items,
    };

    const CLASS_NAME: &str = "RustampPreferences";
    /// Control ids: the close button, then page controls from `FIRST_PAGE_ID`.
    const ID_CLOSE: u16 = 1;
    const FIRST_PAGE_ID: u16 = 100;
    const ID_TREE: u16 = 50;
    const ID_SKINS: u16 = 60;
    const ID_PLUGINS: u16 = 61;
    const ID_PLUGIN_TOGGLE: u16 = 62;
    const ID_PLUGIN_RESCAN: u16 = 63;
    const ID_LIBRARY_ROOTS: u16 = 64;
    const ID_LIBRARY_ADD: u16 = 65;
    const ID_LIBRARY_REMOVE: u16 = 66;
    /// Scrollable width (unscaled pixels) of a plug-in row.
    const PLUGIN_ROW_EXTENT: i32 = 900;
    /// Scrollable width (unscaled pixels) of a library root.
    ///
    /// The floor of the list's scroll range: a row is the path and the time
    /// of the last scan, and the range grows with the widest row.
    const ROOT_ROW_EXTENT: i32 = 1080;

    /// Layout of the frame in unscaled pixels (client area).
    const CLIENT: (i32, i32) = (440, 300);
    const TREE_RECT: (i32, i32, i32, i32) = (8, 8, 150, 252);
    const CLOSE_RECT: (i32, i32, i32, i32) = (8, 266, 150, 26);
    const PAGE_ORIGIN: (i32, i32) = (170, 8);

    /// Button style bits (`BS_*`), which the bindings type differently per
    /// version, kept as plain numbers.
    const BS_PUSHBUTTON: u32 = 0x0;
    const BS_AUTOCHECKBOX: u32 = 0x3;
    const BS_GROUPBOX: u32 = 0x7;
    const SS_LEFT: u32 = 0x0;
    const TVS_HASBUTTONS: u32 = 0x1;
    const TVS_HASLINES: u32 = 0x2;
    const TVS_LINESATROOT: u32 = 0x4;
    const TVS_SHOWSELALWAYS: u32 = 0x20;
    const LBS_NOTIFY: u32 = 0x1;
    const LBN_SELCHANGE: u32 = 1;
    const BN_CLICKED: u32 = 0;
    /// Combo box style bit (`CBS_DROPDOWNLIST`): a list the user picks from,
    /// without an edit field. Kept as a plain number like the styles above.
    const CBS_DROPDOWNLIST: u32 = 0x3;

    /// The dialog's live state. The dialog is a singleton per thread (the UI
    /// thread), so the window procedure and the handle share it here instead
    /// of through a raw pointer stored in the window.
    struct Inner {
        hwnd: HWND,
        tree: HWND,
        font: HFONT,
        dpi: u32,
        node: PrefsNode,
        settings: PrefsSettings,
        /// Controls of the current page: (window, binding).
        page: Vec<HWND>,
        checks: Vec<(HWND, u16, Bind)>,
        /// The drop-down lists of the current page: (window, id, binding).
        combos: Vec<(HWND, u16, ComboBind)>,
        skin_list: Option<HWND>,
        /// The state of the three lists of the current page: the rows they
        /// show, the row the user picked in them and the skin the player
        /// says is in use.
        lists: PageLists,
        /// The plug-in list of the current page.
        plugin_list: Option<HWND>,
        /// The media library's root list of the current page.
        library_list: Option<HWND>,
        actions: Vec<PrefsAction>,
        /// Tree item handles, indexed like [`TREE`].
        items: Vec<isize>,
        /// Set while the code (not the user) changes the selection.
        quiet: bool,
    }

    thread_local! {
        static STATE: RefCell<Option<Inner>> = const { RefCell::new(None) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    fn scale(value: i32, dpi: u32) -> i32 {
        (i64::from(value) * i64::from(dpi) / 96) as i32
    }

    /// The handle to the open dialog. All state lives with the window; the
    /// handle only addresses it.
    #[derive(Debug, Clone, Copy)]
    pub struct PrefsWindow {
        hwnd: isize,
    }

    impl PrefsWindow {
        /// Opens the dialog, owned by `raw_owner`, showing `node`. Call on
        /// the UI thread, outside a window procedure.
        pub fn open(
            raw_owner: isize,
            node: PrefsNode,
            settings: &PrefsSettings,
        ) -> Result<Self, PrefsError> {
            register_class()?;
            let owner = HWND(raw_owner as *mut c_void);
            let class = wide(CLASS_NAME);
            let title = wide("Preferences");
            let style = WS_CAPTION | WS_SYSMENU;
            // SAFETY: plain window creation with NUL-terminated strings that
            // outlive the call; the owner is the player's main window.
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR(class.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    style,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    CLIENT.0,
                    CLIENT.1,
                    if raw_owner == 0 { None } else { Some(owner) },
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| PrefsError::Win32(e.to_string()))?;
            // SAFETY: `hwnd` was just created on this thread.
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let font = message_font(dpi);

            // Size the window so the client area is exactly CLIENT.
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: scale(CLIENT.0, dpi),
                bottom: scale(CLIENT.1, dpi),
            };
            // SAFETY: `frame` is a valid RECT.
            let _ = unsafe {
                AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE(0), dpi)
            };
            // SAFETY: `hwnd` is ours; this only moves and sizes it.
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    frame.right - frame.left,
                    frame.bottom - frame.top,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                )
            };

            let tree = child(
                hwnd,
                WC_TREEVIEWW,
                "",
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | TVS_HASBUTTONS
                    | TVS_HASLINES
                    | TVS_LINESATROOT
                    | TVS_SHOWSELALWAYS,
                WS_EX_CLIENTEDGE.0,
                (TREE_RECT.0, TREE_RECT.1, TREE_RECT.2, TREE_RECT.3),
                ID_TREE,
                dpi,
                font,
            )?;
            child(
                hwnd,
                windows::core::w!("BUTTON"),
                "Close",
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                0,
                (CLOSE_RECT.0, CLOSE_RECT.1, CLOSE_RECT.2, CLOSE_RECT.3),
                ID_CLOSE,
                dpi,
                font,
            )?;

            let items = fill_tree(tree);
            STATE.with(|state| {
                *state.borrow_mut() = Some(Inner {
                    hwnd,
                    tree,
                    font,
                    dpi,
                    node,
                    settings: settings.clone(),
                    page: Vec::new(),
                    checks: Vec::new(),
                    combos: Vec::new(),
                    skin_list: None,
                    lists: PageLists::default(),
                    plugin_list: None,
                    library_list: None,
                    actions: Vec::new(),
                    items,
                    quiet: false,
                });
            });
            let handle = PrefsWindow {
                hwnd: hwnd.0 as isize,
            };
            handle.show_page(node);
            // SAFETY: `hwnd` is ours.
            let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
            // SAFETY: `hwnd` is ours.
            let _ = unsafe { SetForegroundWindow(hwnd) };
            Ok(handle)
        }

        /// Whether the window still exists (the user may have closed it).
        pub fn is_open(&self) -> bool {
            // SAFETY: `IsWindow` accepts any handle value.
            unsafe { IsWindow(Some(HWND(self.hwnd as *mut c_void))) }.as_bool()
                && STATE.with(|state| state.borrow().is_some())
        }

        /// Brings the window to the front.
        pub fn raise(&self) {
            // SAFETY: `SetForegroundWindow` accepts any handle value.
            let _ = unsafe { SetForegroundWindow(HWND(self.hwnd as *mut c_void)) };
        }

        /// Closes the window.
        pub fn close(&self) {
            // SAFETY: destroys our own window on its thread.
            let _ = unsafe { DestroyWindow(HWND(self.hwnd as *mut c_void)) };
        }

        /// Shows `node`'s page and selects its row in the tree.
        pub fn show_page(&self, node: PrefsNode) {
            let Some((hwnd, tree, item, dpi, font, settings)) = STATE.with(|state| {
                let mut guard = state.try_borrow_mut().ok()?;
                let inner = guard.as_mut()?;
                inner.node = node;
                inner.quiet = true;
                let index = TREE.iter().position(|e| e.node == node)?;
                Some((
                    inner.hwnd,
                    inner.tree,
                    inner.items.get(index).copied()?,
                    inner.dpi,
                    inner.font,
                    inner.settings.clone(),
                ))
            }) else {
                return;
            };
            // SAFETY: selecting a row of our own tree view.
            unsafe {
                SendMessageW(
                    tree,
                    TVM_SELECTITEM,
                    Some(WPARAM(TVGN_CARET as usize)),
                    Some(LPARAM(item)),
                );
            }
            build_page(hwnd, node, &settings, dpi, font);
            STATE.with(|state| {
                if let Ok(mut guard) = state.try_borrow_mut()
                    && let Some(inner) = guard.as_mut()
                {
                    inner.quiet = false;
                }
            });
        }

        /// Pushes the player's current settings into the open controls.
        pub fn set_settings(&self, settings: &PrefsSettings) {
            let Ok((checks, combos, skin_list, plugin_list, library_list, node)) =
                STATE.with(|state| {
                    let mut guard = state.try_borrow_mut().map_err(|_| ())?;
                    let inner = guard.as_mut().ok_or(())?;
                    inner.settings = settings.clone();
                    Ok::<_, ()>((
                        inner.checks.clone(),
                        inner.combos.clone(),
                        inner.skin_list,
                        inner.plugin_list,
                        inner.library_list,
                        inner.node,
                    ))
                })
            else {
                return;
            };
            let controls = (checks, skin_list);
            if let Some(list) = plugin_list {
                refresh_plugin_list(list, node, settings);
            }
            if let Some(list) = library_list {
                refresh_root_list(list, settings);
            }
            for (control, _, bind) in controls.0 {
                set_check(control, settings.get(bind));
            }
            for (control, _, bind) in combos {
                set_combo_row(control, settings.combo_row(bind));
            }
            if let Some(list) = controls.1 {
                refresh_skin_list(list, settings);
            }
        }

        /// What the user did since the last call.
        pub fn take_actions(&self) -> Vec<PrefsAction> {
            STATE.with(|state| {
                state
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut guard| guard.as_mut().map(|i| std::mem::take(&mut i.actions)))
                    .unwrap_or_default()
            })
        }
    }

    fn set_check(control: HWND, on: bool) {
        // SAFETY: a check state on our own button.
        unsafe {
            SendMessageW(
                control,
                BM_SETCHECK,
                Some(WPARAM(usize::from(on))),
                Some(LPARAM(0)),
            );
        }
    }

    /// Selects the row of a drop-down list, unless it already shows it (the
    /// per-tick settings push must not close a list the user has open).
    fn set_combo_row(control: HWND, row: usize) {
        // SAFETY: the selection of our own combo box.
        let current =
            unsafe { SendMessageW(control, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0;
        if current < 0 || current as usize != row {
            // SAFETY: selecting a row of our own combo box.
            unsafe {
                SendMessageW(control, CB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
            }
        }
    }

    fn register_class() -> Result<(), PrefsError> {
        let class = wide(CLASS_NAME);
        // SAFETY: standard class registration; the name outlives the call.
        unsafe {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES,
            });
            let module = GetModuleHandleW(None).map_err(|e| PrefsError::Win32(e.to_string()))?;
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: module.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            // A second registration fails with "class exists", which is fine.
            let _ = RegisterClassExW(&wc);
        }
        Ok(())
    }

    fn message_font(dpi: u32) -> HFONT {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        // SAFETY: `metrics` is a valid, correctly sized out-parameter.
        let ok = unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                dpi,
            )
        };
        if ok.is_err() {
            return HFONT::default();
        }
        // SAFETY: creates a font from the logfont the OS just filled in.
        unsafe { CreateFontIndirectW(&metrics.lfMessageFont) }
    }

    #[allow(clippy::too_many_arguments)]
    fn child(
        parent: HWND,
        class: PCWSTR,
        text: &str,
        style: u32,
        ex_style: u32,
        (x, y, w, h): (i32, i32, i32, i32),
        id: u16,
        dpi: u32,
        font: HFONT,
    ) -> Result<HWND, PrefsError> {
        let text = wide(text);
        // SAFETY: creating a child control of our own window.
        let control = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(ex_style),
                class,
                PCWSTR(text.as_ptr()),
                WINDOW_STYLE(style),
                scale(x, dpi),
                scale(y, dpi),
                scale(w, dpi),
                scale(h, dpi),
                Some(parent),
                Some(HMENU(usize::from(id) as *mut c_void)),
                None,
                None,
            )
        }
        .map_err(|e| PrefsError::Win32(e.to_string()))?;
        // SAFETY: setting the font of a control we own.
        unsafe {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        Ok(control)
    }

    /// Fills the tree view; returns each row's item handle, in [`TREE`] order.
    fn fill_tree(tree: HWND) -> Vec<isize> {
        let mut handles: Vec<isize> = Vec::new();
        let mut last_root = 0isize;
        for (index, entry) in TREE.iter().enumerate() {
            let mut text = wide(entry.label);
            let parent = if entry.depth == 0 { 0 } else { last_root };
            let insert = TVINSERTSTRUCTW {
                hParent: windows::Win32::UI::Controls::HTREEITEM(parent),
                hInsertAfter: TVI_LAST,
                Anonymous: windows::Win32::UI::Controls::TVINSERTSTRUCTW_0 {
                    item: TVITEMW {
                        mask: TVIF_TEXT | TVIF_PARAM,
                        pszText: PWSTR(text.as_mut_ptr()),
                        lParam: LPARAM(index as isize),
                        ..Default::default()
                    },
                },
            };
            // SAFETY: the insert struct and its text outlive the call.
            let item = unsafe {
                SendMessageW(
                    tree,
                    TVM_INSERTITEMW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&insert as *const TVINSERTSTRUCTW as isize)),
                )
            }
            .0;
            if entry.depth == 0 {
                last_root = item;
            }
            handles.push(item);
        }
        // Open every root so the whole tree is visible, like the reference.
        for (index, entry) in TREE.iter().enumerate() {
            if entry.depth == 0 {
                // SAFETY: expanding a row of our own tree view.
                unsafe {
                    SendMessageW(
                        tree,
                        TVM_EXPAND,
                        Some(WPARAM(TVE_EXPAND.0 as usize)),
                        Some(LPARAM(handles[index])),
                    );
                }
            }
        }
        handles
    }

    /// Replaces the page area's controls with `node`'s page.
    fn build_page(hwnd: HWND, node: PrefsNode, settings: &PrefsSettings, dpi: u32, font: HFONT) {
        let old = STATE.with(|state| {
            state
                .try_borrow_mut()
                .ok()
                .and_then(|mut g| {
                    g.as_mut().map(|i| {
                        i.checks.clear();
                        i.combos.clear();
                        i.skin_list = None;
                        i.plugin_list = None;
                        i.library_list = None;
                        // The lists are new controls: the rows of the old
                        // ones say nothing about them, so the new lists are
                        // filled again.
                        i.lists = PageLists::shown(node);
                        std::mem::take(&mut i.page)
                    })
                })
                .unwrap_or_default()
        });
        for control in old {
            // SAFETY: destroying a control we created.
            let _ = unsafe { DestroyWindow(control) };
        }
        let shift = |r: Rect| (r.x + PAGE_ORIGIN.0, r.y + PAGE_ORIGIN.1, r.w, r.h);
        let mut page = Vec::new();
        let mut checks = Vec::new();
        let mut combos = Vec::new();
        let mut skin_list = None;
        let mut plugin_list = None;
        let mut library_list = None;
        for (next_id, item) in (FIRST_PAGE_ID..).zip(page_items(node)) {
            match item {
                Item::Group { text, rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | BS_GROUPBOX,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::Label { text, rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("STATIC"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::Check { text, rect, bind } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX,
                        0,
                        shift(rect),
                        next_id,
                        dpi,
                        font,
                    ) {
                        set_check(c, settings.get(bind));
                        page.push(c);
                        checks.push((c, next_id, bind));
                    }
                }
                Item::Combo {
                    text,
                    label,
                    list,
                    bind,
                } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("STATIC"),
                        text,
                        WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT,
                        0,
                        shift(label),
                        next_id,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("COMBOBOX"),
                        "",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST,
                        0,
                        shift(list),
                        next_id,
                        dpi,
                        font,
                    ) {
                        for entry in bind.labels() {
                            let entry = wide(entry);
                            // SAFETY: the text outlives the call.
                            unsafe {
                                SendMessageW(
                                    c,
                                    CB_ADDSTRING,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(entry.as_ptr() as isize)),
                                );
                            }
                        }
                        set_combo_row(c, settings.combo_row(bind));
                        page.push(c);
                        combos.push((c, next_id, bind));
                    }
                }
                Item::SkinList { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("LISTBOX"),
                        "",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | LBS_NOTIFY,
                        WS_EX_CLIENTEDGE.0,
                        shift(rect),
                        ID_SKINS,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                        skin_list = Some(c);
                    }
                }
                Item::PluginList { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("LISTBOX"),
                        "",
                        WS_CHILD.0
                            | WS_VISIBLE.0
                            | WS_TABSTOP.0
                            | WS_VSCROLL.0
                            | WS_HSCROLL.0
                            | LBS_NOTIFY,
                        WS_EX_CLIENTEDGE.0,
                        shift(rect),
                        ID_PLUGINS,
                        dpi,
                        font,
                    ) {
                        // Rows carry a file name, the approval verdict and
                        // possibly a refusal reason: let the list scroll
                        // sideways instead of cutting them off.
                        // SAFETY: sizing the scroll range of our own list box.
                        unsafe {
                            SendMessageW(
                                c,
                                LB_SETHORIZONTALEXTENT,
                                Some(WPARAM(scale(PLUGIN_ROW_EXTENT, dpi) as usize)),
                                Some(LPARAM(0)),
                            );
                        }
                        page.push(c);
                        plugin_list = Some(c);
                    }
                }
                Item::PluginToggle { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        "Switch on / off",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                        0,
                        shift(rect),
                        ID_PLUGIN_TOGGLE,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::PluginRescan { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        "Rescan",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                        0,
                        shift(rect),
                        ID_PLUGIN_RESCAN,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::LibraryRoots { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("LISTBOX"),
                        "",
                        WS_CHILD.0
                            | WS_VISIBLE.0
                            | WS_TABSTOP.0
                            | WS_VSCROLL.0
                            | WS_HSCROLL.0
                            | LBS_NOTIFY,
                        WS_EX_CLIENTEDGE.0,
                        shift(rect),
                        ID_LIBRARY_ROOTS,
                        dpi,
                        font,
                    ) {
                        // A row is a path, and a path can be longer than the
                        // list: let the list scroll sideways instead of
                        // cutting the row off. The range grows with the
                        // widest row once the rows are in.
                        // SAFETY: sizing the scroll range of our own list box.
                        unsafe {
                            SendMessageW(
                                c,
                                LB_SETHORIZONTALEXTENT,
                                Some(WPARAM(scale(ROOT_ROW_EXTENT, dpi) as usize)),
                                Some(LPARAM(0)),
                            );
                        }
                        page.push(c);
                        library_list = Some(c);
                    }
                }
                Item::LibraryAdd { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        "Add folder...",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                        0,
                        shift(rect),
                        ID_LIBRARY_ADD,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
                Item::LibraryRemove { rect } => {
                    if let Ok(c) = child(
                        hwnd,
                        windows::core::w!("BUTTON"),
                        "Remove",
                        WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON,
                        0,
                        shift(rect),
                        ID_LIBRARY_REMOVE,
                        dpi,
                        font,
                    ) {
                        page.push(c);
                    }
                }
            }
        }
        STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut()
                && let Some(inner) = guard.as_mut()
            {
                inner.page = page;
                inner.checks = checks;
                inner.combos = combos;
                inner.skin_list = skin_list;
                inner.plugin_list = plugin_list;
                inner.library_list = library_list;
            }
        });
        if let Some(list) = plugin_list {
            refresh_plugin_list(list, node, settings);
        }
        if let Some(list) = library_list {
            refresh_root_list(list, settings);
        }
        if let Some(list) = skin_list {
            refresh_skin_list(list, settings);
        }
    }

    /// Makes the skin list show `settings`' skins, keeping the user's pick.
    ///
    /// Does nothing when the list already shows exactly them, so the
    /// per-tick settings push does not flicker the list.
    ///
    /// The rows can change while the dialog is open (a skin that appears in
    /// the folder, or the skin in use), so the list is refilled when the
    /// content changed - not only when the page was built.
    fn refresh_skin_list(list: HWND, settings: &PrefsSettings) {
        let Some((update, texts)) = STATE.with(|state| {
            let mut guard = state.try_borrow_mut().ok()?;
            let inner = guard.as_mut()?;
            let update = inner.lists.skins_mut().apply(settings);
            let texts = inner.lists.skins().texts().to_vec();
            Some((update, texts))
        }) else {
            return;
        };
        if update.refill {
            // SAFETY: refilling our own list box.
            unsafe {
                SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
                for skin in &texts {
                    let wide_text = wide(skin);
                    SendMessageW(
                        list,
                        LB_ADDSTRING,
                        Some(WPARAM(0)),
                        Some(LPARAM(wide_text.as_ptr() as isize)),
                    );
                }
            }
        }
        if let Some(row) = update.selected {
            // SAFETY: selecting a row of our own list box.
            unsafe {
                SendMessageW(list, LB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
            }
        }
    }

    /// Makes the plug-in list show `settings`' rows for `node`, keeping the
    /// selected row. Does nothing when the list already shows exactly them, so
    /// the per-tick settings push does not flicker the list.
    ///
    /// The rows belong to the page: the list of another page is never the
    /// list of this one, so a page switch always refills the list.
    fn refresh_plugin_list(list: HWND, node: PrefsNode, settings: &PrefsSettings) {
        // The user's pick is in the control: the model must know it before
        // the rows change under it.
        // SAFETY: querying our own list box.
        let picked =
            unsafe { SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0;
        let Some((update, texts)) = STATE.with(|state| {
            let mut guard = state.try_borrow_mut().ok()?;
            let inner = guard.as_mut()?;
            if let Ok(row) = usize::try_from(picked) {
                inner.lists.plugins_mut().pick(row);
            }
            let update = inner.lists.plugins_mut().apply(settings, node);
            let texts = inner.lists.plugins().texts().to_vec();
            Some((update, texts))
        }) else {
            return;
        };
        if update.refill {
            // SAFETY: refilling our own list box.
            unsafe {
                SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
                for text in &texts {
                    let wide_text = wide(text);
                    SendMessageW(
                        list,
                        LB_ADDSTRING,
                        Some(WPARAM(0)),
                        Some(LPARAM(wide_text.as_ptr() as isize)),
                    );
                }
            }
        }
        if let Some(row) = update.selected {
            // SAFETY: selecting a row of our own list box.
            unsafe {
                SendMessageW(list, LB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
            }
        }
    }

    /// Makes the media library's root list show `settings`' roots, keeping the
    /// selected row. Does nothing when the list already shows exactly them, so
    /// the per-tick settings push does not flicker the list.
    fn refresh_root_list(list: HWND, settings: &PrefsSettings) {
        // The user's pick is in the control: the model must know it before
        // the rows change under it.
        // SAFETY: querying our own list box.
        let picked =
            unsafe { SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0;
        let Some((update, texts)) = STATE.with(|state| {
            let mut guard = state.try_borrow_mut().ok()?;
            let inner = guard.as_mut()?;
            if let Ok(row) = usize::try_from(picked) {
                inner.lists.roots_mut().pick(row);
            }
            let update = inner.lists.roots_mut().apply(settings);
            let texts = inner.lists.roots().texts().to_vec();
            Some((update, texts))
        }) else {
            return;
        };
        if update.refill {
            // SAFETY: refilling our own list box.
            unsafe {
                SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
                for text in &texts {
                    let wide_text = wide(text);
                    SendMessageW(
                        list,
                        LB_ADDSTRING,
                        Some(WPARAM(0)),
                        Some(LPARAM(wide_text.as_ptr() as isize)),
                    );
                }
            }
            // The rows are one line each, and the list scrolls sideways: the
            // scroll range must reach the end of the longest row, or a long path
            // is cut off.
            set_row_extent(list, &texts);
        }
        if let Some(row) = update.selected {
            // SAFETY: selecting a row of our own list box.
            unsafe {
                SendMessageW(list, LB_SETCURSEL, Some(WPARAM(row)), Some(LPARAM(0)));
            }
        }
    }

    /// Makes the list scroll sideways as far as its widest row.
    ///
    /// The range is never smaller than [`ROOT_ROW_EXTENT`], so the list looks
    /// the same as it did, and it is never smaller than the widest row, so
    /// the row stays readable.
    fn set_row_extent(list: HWND, texts: &[String]) {
        let dpi = STATE.with(|state| {
            state
                .try_borrow()
                .ok()
                .and_then(|g| g.as_ref().map(|i| i.dpi))
                .unwrap_or(96)
        });
        let measured = widest_row(list, texts).unwrap_or(0);
        let extent = scale(ROOT_ROW_EXTENT, dpi).max(measured + 8);
        // SAFETY: sizing the scroll range of our own list box.
        unsafe {
            SendMessageW(
                list,
                LB_SETHORIZONTALEXTENT,
                Some(WPARAM(extent as usize)),
                Some(LPARAM(0)),
            );
        }
    }

    /// The width, in pixels, of the widest row, measured with the list's own
    /// font.
    ///
    /// `None` when the width could not be measured: the caller then keeps the
    /// constant extent.
    fn widest_row(list: HWND, texts: &[String]) -> Option<i32> {
        // SAFETY: the list is our own window, so its device context is ours
        // to take and to release again. The font of the list is the one the
        // rows are drawn with.
        unsafe {
            let dc = GetDC(Some(list));
            if dc.is_invalid() {
                return None;
            }
            let mut widest = 0;
            for text in texts {
                let wide_text = wide(text);
                let mut size = SIZE::default();
                if GetTextExtentPoint32W(dc, &wide_text, &mut size).as_bool() {
                    widest = widest.max(size.cx);
                }
            }
            let _ = ReleaseDC(Some(list), dc);
            Some(widest)
        }
    }

    /// Records a user action from the window procedure (queue only: no
    /// player state is touched here).
    fn push_action(action: PrefsAction) {
        STATE.with(|state| {
            if let Ok(mut guard) = state.try_borrow_mut()
                && let Some(inner) = guard.as_mut()
            {
                inner.actions.push(action);
            }
        });
    }

    // SAFETY: Windows calls this with OS-provided values only; pointers in
    // `lparam` are valid for the matching notification and read as such.
    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                if id == ID_CLOSE && code == BN_CLICKED {
                    // SAFETY: closes our own window.
                    let _ = unsafe { DestroyWindow(hwnd) };
                    return LRESULT(0);
                }
                if id == ID_PLUGIN_RESCAN && code == BN_CLICKED {
                    push_action(PrefsAction::RescanPlugins);
                    return LRESULT(0);
                }
                if id == ID_LIBRARY_ADD && code == BN_CLICKED {
                    // The dialog does not ask for the folder itself: the
                    // player does, from the tick.
                    push_action(PrefsAction::AddLibraryRoot);
                    return LRESULT(0);
                }
                if id == ID_LIBRARY_REMOVE && code == BN_CLICKED {
                    // The row the user picked; no selection removes nothing.
                    let picked = STATE.with(|state| {
                        let guard = state.try_borrow().ok()?;
                        let inner = guard.as_ref()?;
                        let list = inner.library_list?;
                        // SAFETY: querying our own list box.
                        let row = unsafe {
                            SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                        }
                        .0;
                        usize::try_from(row).ok()
                    });
                    if let Some(row) = picked {
                        push_action(PrefsAction::RemoveLibraryRoot(row));
                    }
                    return LRESULT(0);
                }
                if id == ID_PLUGIN_TOGGLE && code == BN_CLICKED {
                    // The id behind the selected row; an information row or no
                    // selection toggles nothing.
                    let picked = STATE.with(|state| {
                        let guard = state.try_borrow().ok()?;
                        let inner = guard.as_ref()?;
                        let list = inner.plugin_list?;
                        // SAFETY: querying our own list box.
                        let row = unsafe {
                            SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                        }
                        .0;
                        usize::try_from(row)
                            .ok()
                            .and_then(|row| inner.lists.plugins().id(row))
                    });
                    if let Some(plugin) = picked {
                        push_action(PrefsAction::TogglePlugin(plugin));
                    }
                    return LRESULT(0);
                }
                if id == ID_SKINS && code == LBN_SELCHANGE {
                    let list = HWND(lparam.0 as *mut c_void);
                    // SAFETY: querying our own list box.
                    let row = unsafe {
                        SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                    }
                    .0;
                    if row >= 0 {
                        let row = row as usize;
                        // The pick belongs to the list's state: the player's
                        // push must not take it back before the player got
                        // it.
                        STATE.with(|state| {
                            if let Ok(mut guard) = state.try_borrow_mut()
                                && let Some(inner) = guard.as_mut()
                            {
                                inner.lists.skins_mut().pick(row);
                            }
                        });
                        push_action(PrefsAction::LoadSkin(row));
                    }
                    return LRESULT(0);
                }
                if code == CBN_SELCHANGE {
                    // A drop-down list of the current page picked another row.
                    let hit = STATE.with(|state| {
                        state.try_borrow().ok().and_then(|g| {
                            g.as_ref().and_then(|i| {
                                i.combos.iter().find(|(_, cid, _)| *cid == id).copied()
                            })
                        })
                    });
                    if let Some((control, _, bind)) = hit {
                        // SAFETY: reading the selection of our own combo box.
                        let row = unsafe {
                            SendMessageW(control, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)))
                        }
                        .0;
                        if row >= 0
                            && let Some(action) = bind.action(row as usize)
                        {
                            push_action(action);
                        }
                        return LRESULT(0);
                    }
                }
                if code == BN_CLICKED {
                    let hit = STATE.with(|state| {
                        state.try_borrow().ok().and_then(|g| {
                            g.as_ref().and_then(|i| {
                                i.checks.iter().find(|(_, cid, _)| *cid == id).copied()
                            })
                        })
                    });
                    if let Some((control, _, bind)) = hit {
                        // SAFETY: reading the state of our own check box.
                        let on = unsafe {
                            SendMessageW(control, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0)))
                        }
                        .0 == 1;
                        push_action(PrefsAction::Set(bind, on));
                        return LRESULT(0);
                    }
                }
            }
            WM_NOTIFY => {
                // SAFETY: for a tree-view notification `lparam` points at an
                // NMTREEVIEWW that lives for the duration of the message.
                let header = unsafe { &*(lparam.0 as *const NMHDR) };
                if header.idFrom == usize::from(ID_TREE) && header.code == TVN_SELCHANGEDW {
                    let notify = unsafe { &*(lparam.0 as *const NMTREEVIEWW) };
                    let index = notify.itemNew.lParam.0 as usize;
                    let quiet = STATE.with(|state| {
                        state
                            .try_borrow()
                            .ok()
                            .and_then(|g| g.as_ref().map(|i| i.quiet))
                            .unwrap_or(true)
                    });
                    if !quiet && let Some(entry) = TREE.get(index) {
                        // Rebuilding destroys controls, which can send this
                        // window messages: do it from a clean borrow state.
                        let info = STATE.with(|state| {
                            state.try_borrow().ok().and_then(|g| {
                                g.as_ref().map(|i| (i.dpi, i.font, i.settings.clone()))
                            })
                        });
                        if let Some((dpi, font, settings)) = info {
                            STATE.with(|state| {
                                if let Ok(mut g) = state.try_borrow_mut()
                                    && let Some(i) = g.as_mut()
                                {
                                    i.node = entry.node;
                                    i.quiet = true;
                                }
                            });
                            build_page(hwnd, entry.node, &settings, dpi, font);
                            STATE.with(|state| {
                                if let Ok(mut g) = state.try_borrow_mut()
                                    && let Some(i) = g.as_mut()
                                {
                                    i.quiet = false;
                                }
                            });
                        }
                    }
                }
            }
            WM_DESTROY => {
                let font = STATE.with(|state| {
                    state
                        .try_borrow_mut()
                        .ok()
                        .and_then(|mut g| g.take().map(|i| i.font))
                });
                if let Some(font) = font
                    && !font.is_invalid()
                {
                    // SAFETY: deleting the font this dialog created.
                    let _ = unsafe { DeleteObject(font.into()) };
                }
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: default handling for everything else.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod props;

    #[test]
    fn the_tree_lists_the_reference_pages_with_children_after_their_parent() {
        let labels: Vec<_> = TREE.iter().map(|e| e.label).collect();
        assert_eq!(
            labels,
            [
                "Setup",
                "File types",
                "Options",
                "Display",
                "Visualization",
                "Shuffle",
                "Plug-ins",
                "Input",
                "Output",
                "Visualization",
                "DSP/Effect",
                "General Purpose",
                "Skins",
                "Library",
                "Bookmarks",
            ]
        );
        assert_eq!(TREE[0].depth, 0, "the first row is a root");
        assert!(TREE.iter().all(|e| e.depth <= 1));
    }

    #[test]
    fn every_node_has_exactly_one_row_and_a_page() {
        for entry in TREE {
            assert_eq!(
                TREE.iter().filter(|e| e.node == entry.node).count(),
                1,
                "{:?}",
                entry.node
            );
            assert!(!page_items(entry.node).is_empty(), "{:?}", entry.node);
        }
    }

    #[test]
    fn every_item_fits_inside_the_page_area() {
        for entry in TREE {
            for item in page_items(entry.node) {
                let rects: Vec<Rect> = match item {
                    Item::Group { rect, .. }
                    | Item::Label { rect, .. }
                    | Item::Check { rect, .. }
                    | Item::SkinList { rect }
                    | Item::PluginList { rect }
                    | Item::PluginToggle { rect }
                    | Item::PluginRescan { rect }
                    | Item::LibraryRoots { rect }
                    | Item::LibraryAdd { rect }
                    | Item::LibraryRemove { rect } => vec![rect],
                    Item::Combo { label, list, .. } => vec![label, list],
                };
                for r in rects {
                    assert!(r.x >= 0 && r.y >= 0, "{:?}: {r:?}", entry.node);
                    assert!(r.x + r.w <= PAGE_SIZE.0, "{:?}: {r:?}", entry.node);
                    assert!(r.y + r.h <= PAGE_SIZE.1, "{:?}: {r:?}", entry.node);
                }
            }
        }
    }

    #[test]
    fn each_setting_has_exactly_one_check_box() {
        let mut seen: Vec<Bind> = Vec::new();
        for entry in TREE {
            for item in page_items(entry.node) {
                if let Item::Check { bind, .. } = item {
                    assert!(!seen.contains(&bind), "{bind:?} appears twice");
                    seen.push(bind);
                }
            }
        }
        for bind in [
            Bind::AlwaysOnTop,
            Bind::EasyMove,
            Bind::DoubleSize,
            Bind::Shuffle,
            Bind::Repeat,
        ] {
            assert!(seen.contains(&bind), "{bind:?} has no control");
        }
    }

    #[test]
    fn each_list_setting_has_exactly_one_drop_down() {
        let mut seen: Vec<ComboBind> = Vec::new();
        for entry in TREE {
            for item in page_items(entry.node) {
                if let Item::Combo { bind, .. } = item {
                    assert!(!seen.contains(&bind), "{bind:?} appears twice");
                    seen.push(bind);
                }
            }
        }
        for bind in [ComboBind::ReplayGain, ComboBind::OutputMode] {
            assert!(seen.contains(&bind), "{bind:?} has no control");
        }
    }

    #[test]
    fn the_setup_page_carries_the_two_audio_settings() {
        // The reference's page contents are unmeasured (S04 is open), so the
        // page that carries them is an own choice: Setup is the page for the
        // device, and it is the one that says what the player does.
        let items = page_items(PrefsNode::Setup);
        let combos: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                Item::Combo { text, bind, .. } => Some((*text, *bind)),
                _ => None,
            })
            .collect();
        assert_eq!(
            combos,
            [
                ("ReplayGain:", ComboBind::ReplayGain),
                ("Output mode:", ComboBind::OutputMode),
            ]
        );
    }

    #[test]
    fn the_replaygain_list_shows_every_mode_in_order() {
        assert_eq!(
            REPLAYGAIN_CHOICES,
            [
                ReplayGainMode::Off,
                ReplayGainMode::Track,
                ReplayGainMode::Album
            ]
        );
        assert_eq!(
            ComboBind::ReplayGain.labels(),
            vec!["Off", "Track", "Album"]
        );
        // Every mode has a row, and the row shows that mode again.
        for (row, mode) in REPLAYGAIN_CHOICES.iter().copied().enumerate() {
            assert_eq!(replaygain_row(mode), row, "{mode:?}");
            assert_eq!(replaygain_at(row), Some(mode), "row {row}");
        }
        assert_eq!(replaygain_at(REPLAYGAIN_CHOICES.len()), None);
    }

    #[test]
    fn the_output_mode_list_shows_every_mode_in_order() {
        assert_eq!(
            OUTPUT_MODE_CHOICES,
            [OutputMode::Shared, OutputMode::Exclusive]
        );
        assert_eq!(ComboBind::OutputMode.labels(), vec!["Shared", "Exclusive"]);
        for (row, mode) in OUTPUT_MODE_CHOICES.iter().copied().enumerate() {
            assert_eq!(output_mode_row(mode), row, "{mode:?}");
            assert_eq!(output_mode_at(row), Some(mode), "row {row}");
        }
        assert_eq!(output_mode_at(OUTPUT_MODE_CHOICES.len()), None);
    }

    #[test]
    fn a_list_row_stands_for_the_action_that_sets_its_value() {
        for (row, mode) in REPLAYGAIN_CHOICES.iter().copied().enumerate() {
            assert_eq!(
                ComboBind::ReplayGain.action(row),
                Some(PrefsAction::SetReplayGain(mode))
            );
        }
        for (row, mode) in OUTPUT_MODE_CHOICES.iter().copied().enumerate() {
            assert_eq!(
                ComboBind::OutputMode.action(row),
                Some(PrefsAction::SetOutputMode(mode))
            );
        }
        // A row that does not exist changes nothing.
        assert_eq!(ComboBind::ReplayGain.action(REPLAYGAIN_CHOICES.len()), None);
        assert_eq!(
            ComboBind::OutputMode.action(OUTPUT_MODE_CHOICES.len()),
            None
        );
    }

    #[test]
    fn the_settings_show_the_row_of_their_value() {
        let settings = PrefsSettings {
            replaygain: ReplayGainMode::Album,
            output_mode: OutputMode::Exclusive,
            ..PrefsSettings::default()
        };
        assert_eq!(settings.combo_row(ComboBind::ReplayGain), 2);
        assert_eq!(settings.combo_row(ComboBind::OutputMode), 1);
    }

    #[test]
    fn the_default_settings_show_the_first_row() {
        let settings = PrefsSettings::default();
        assert_eq!(settings.combo_row(ComboBind::ReplayGain), 0);
        assert_eq!(settings.combo_row(ComboBind::OutputMode), 0);
        assert_eq!(settings.replaygain, ReplayGainMode::Off);
        assert_eq!(settings.output_mode, OutputMode::Shared);
    }

    #[test]
    fn a_value_survives_the_way_through_the_dialog() {
        // The whole path of a change: the configuration's value becomes the
        // list's row, the row becomes the action, and the action carries the
        // value back to the configuration.
        for mode in [
            ReplayGainMode::Off,
            ReplayGainMode::Track,
            ReplayGainMode::Album,
        ] {
            let settings = PrefsSettings {
                replaygain: mode,
                ..PrefsSettings::default()
            };
            let row = settings.combo_row(ComboBind::ReplayGain);
            let action = ComboBind::ReplayGain
                .action(row)
                .expect("a row stands for a mode");
            assert_eq!(action, PrefsAction::SetReplayGain(mode));
        }
        for mode in [OutputMode::Shared, OutputMode::Exclusive] {
            let settings = PrefsSettings {
                output_mode: mode,
                ..PrefsSettings::default()
            };
            let row = settings.combo_row(ComboBind::OutputMode);
            let action = ComboBind::OutputMode
                .action(row)
                .expect("a row stands for a mode");
            assert_eq!(action, PrefsAction::SetOutputMode(mode));
        }
    }

    #[test]
    fn every_plugin_page_has_a_list_a_toggle_and_a_rescan_button() {
        for node in [
            PrefsNode::Input,
            PrefsNode::Output,
            PrefsNode::PlugInVisualization,
            PrefsNode::DspEffect,
            PrefsNode::GeneralPurpose,
        ] {
            let items = page_items(node);
            let count = |f: fn(&Item) -> bool| items.iter().filter(|i| f(i)).count();
            assert_eq!(
                count(|i| matches!(i, Item::PluginList { .. })),
                1,
                "{node:?}"
            );
            assert_eq!(
                count(|i| matches!(i, Item::PluginToggle { .. })),
                1,
                "{node:?}"
            );
            assert_eq!(
                count(|i| matches!(i, Item::PluginRescan { .. })),
                1,
                "{node:?}"
            );
        }
    }

    #[test]
    fn the_plugin_hint_states_the_approval_gate_and_the_startup_load() {
        let hint = PLUGIN_HINT;
        assert!(hint.contains("approved"), "{hint}");
        assert!(
            hint.contains("loaded when the plugin host starts"),
            "{hint}"
        );
    }

    #[test]
    fn plugin_rows_are_filtered_by_page_in_order() {
        let row = |node, id, text: &str| PluginRow {
            node,
            id,
            text: text.to_string(),
        };
        let settings = PrefsSettings {
            plugins: vec![
                row(PrefsNode::Input, Some(0), "in_a"),
                row(PrefsNode::GeneralPurpose, Some(1), "gen_a"),
                row(PrefsNode::Input, None, "note"),
            ],
            ..PrefsSettings::default()
        };
        let input: Vec<_> = settings
            .plugin_rows(PrefsNode::Input)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(input, ["in_a", "note"]);
        assert_eq!(settings.plugin_rows(PrefsNode::DspEffect).count(), 0);
    }

    /// The pages whose list shows plug-ins, in the order of the tree.
    fn plugin_pages() -> Vec<PrefsNode> {
        TREE.iter()
            .map(|entry| entry.node)
            .filter(|node| {
                page_items(*node)
                    .iter()
                    .any(|item| matches!(item, Item::PluginList { .. }))
            })
            .collect()
    }

    /// Settings with one plug-in row on every plug-in page.
    fn settings_with_plugin_rows() -> PrefsSettings {
        PrefsSettings {
            plugins: plugin_pages()
                .iter()
                .enumerate()
                .map(|(index, node)| PluginRow {
                    node: *node,
                    id: Some(index),
                    text: format!("{node:?} plug-in"),
                })
                .collect(),
            ..PrefsSettings::default()
        }
    }

    #[test]
    fn five_pages_list_plug_ins() {
        let pages = plugin_pages();
        assert_eq!(pages.len(), 5, "{pages:?}");
        assert_eq!(
            pages,
            [
                PrefsNode::Input,
                PrefsNode::Output,
                PrefsNode::PlugInVisualization,
                PrefsNode::DspEffect,
                PrefsNode::GeneralPurpose,
            ]
        );
    }

    #[test]
    fn the_plugin_list_of_a_page_shows_the_rows_of_that_page() {
        let settings = settings_with_plugin_rows();
        for node in plugin_pages() {
            let state = PluginListState::of(&settings, node);
            let rows: Vec<(Option<usize>, String)> = settings
                .plugin_rows(node)
                .map(|row| (row.id, row.text.clone()))
                .collect();
            let texts: Vec<String> = rows.iter().map(|(_, text)| text.clone()).collect();
            assert_eq!(state.texts(), texts, "{node:?}");
            for (row, (id, _)) in rows.iter().enumerate() {
                assert_eq!(state.id(row), *id, "{node:?}: row {row}");
            }
        }
    }

    #[test]
    fn a_page_switch_refills_the_plugin_list_even_when_the_rows_are_the_same() {
        // The player says the same thing on every plug-in page when it has
        // nothing to list, so two pages can have the very same rows. The
        // list of one page must not pass for the list of another one: it
        // would keep the rows of the page the user came from.
        let same = "No plug-ins of this type in C:\\Plugins";
        let settings = PrefsSettings {
            plugins: plugin_pages()
                .iter()
                .map(|node| PluginRow {
                    node: *node,
                    id: None,
                    text: same.to_string(),
                })
                .collect(),
            ..PrefsSettings::default()
        };
        for node in plugin_pages() {
            let state = PluginListState::of(&settings, node);
            for other in plugin_pages() {
                if other == node {
                    continue;
                }
                let other_state = PluginListState::of(&settings, other);
                assert_eq!(state.texts(), other_state.texts(), "the rows are the same");
                assert!(
                    !state.shows(&other_state),
                    "the list of {node:?} must not pass for the list of {other:?}"
                );
            }
        }
        // Two pages without a single row are not the same list either.
        let empty = PrefsSettings::default();
        for node in plugin_pages() {
            let state = PluginListState::of(&empty, node);
            assert!(state.texts().is_empty());
            for other in plugin_pages() {
                if other == node {
                    continue;
                }
                assert!(
                    !state.shows(&PluginListState::of(&empty, other)),
                    "the empty list of {node:?} must not pass for the empty list of {other:?}"
                );
            }
        }
    }

    #[test]
    fn a_list_that_showed_nothing_is_not_the_list_of_a_page() {
        let settings = settings_with_plugin_rows();
        let empty = PluginListState::default();
        for node in plugin_pages() {
            assert!(
                !empty.shows(&PluginListState::of(&settings, node)),
                "{node:?}: a list that showed nothing shows nothing of this page"
            );
        }
    }

    #[test]
    fn the_id_behind_a_row_is_the_id_of_the_row_of_that_page() {
        let settings = settings_with_plugin_rows();
        let pages = plugin_pages();
        for (index, node) in pages.iter().enumerate() {
            let state = PluginListState::of(&settings, *node);
            assert_eq!(state.id(0), Some(index), "{node:?}: the first row");
            assert_eq!(state.id(pages.len()), None, "{node:?}: no such row");
        }
    }

    /// Settings with the skins `skins` and the skin in use in the row
    /// `selected`.
    fn skin_settings(skins: &[&str], selected: Option<usize>) -> PrefsSettings {
        PrefsSettings {
            skins: skins.iter().map(|skin| skin.to_string()).collect(),
            skin_selected: selected,
            ..PrefsSettings::default()
        }
    }

    #[test]
    fn the_skin_list_refills_only_when_the_rows_change() {
        let settings = skin_settings(&["a", "b"], Some(0));
        let mut state = SkinListState::default();
        // The page build leaves the list empty: the first push fills it.
        let update = state.apply(&settings);
        assert!(update.refill, "the first push fills the list");
        assert_eq!(update.selected, Some(0), "the skin in use is selected");
        // The same rows: the list must not flicker.
        let update = state.apply(&settings);
        assert!(!update.refill, "the same rows do not refill the list");
        assert_eq!(update.selected, Some(0), "the selection stays");
        // A skin that appeared in the folder: the rows changed.
        let more = skin_settings(&["a", "b", "c"], Some(0));
        let update = state.apply(&more);
        assert!(update.refill, "a new skin refills the list");
        assert_eq!(update.selected, Some(0), "the selection stays");
        // A skin that left the folder: the rows changed.
        let fewer = skin_settings(&["a", "c"], Some(0));
        let update = state.apply(&fewer);
        assert!(update.refill, "a removed skin refills the list");
    }

    #[test]
    fn a_selection_survives_a_change_of_the_rows() {
        let settings = skin_settings(&["a", "b"], Some(1));
        let mut state = SkinListState::default();
        state.apply(&settings);
        // A new skin appears in the folder: the row of the skin in use
        // moves, but the skin in use stays selected.
        let more = skin_settings(&["a", "b", "c"], Some(1));
        let update = state.apply(&more);
        assert!(update.refill, "the rows changed");
        assert_eq!(update.selected, Some(1), "the skin in use stays selected");
        // The skin in use is the first row now, and it stays selected.
        let first = skin_settings(&["b", "c"], Some(0));
        let update = state.apply(&first);
        assert!(update.refill, "the rows changed");
        assert_eq!(update.selected, Some(0), "the skin in use stays selected");
    }

    #[test]
    fn a_vanished_selection_falls_back_to_the_first_row() {
        let settings = skin_settings(&["a", "b"], Some(0));
        let mut state = SkinListState::default();
        state.apply(&settings);
        // The user picks the second skin; the player has not got the pick
        // yet.
        state.pick(1);
        // The picked skin leaves the folder before the player takes the
        // pick: the first row takes its place.
        let fewer = skin_settings(&["a"], Some(0));
        let update = state.apply(&fewer);
        assert!(update.refill, "the rows changed");
        assert_eq!(update.selected, Some(0), "the first row took the place");
        // With no row at all, nothing is selected.
        let none = skin_settings(&[], None);
        let update = state.apply(&none);
        assert!(update.refill, "the rows changed");
        assert_eq!(update.selected, None, "no row took the place");
    }

    #[test]
    fn the_push_does_not_take_back_the_user_s_pick() {
        let settings = skin_settings(&["a", "b"], Some(0));
        let mut state = SkinListState::default();
        state.apply(&settings);
        // The user picks the second row; the player still says that the
        // first skin is in use.
        state.pick(1);
        let update = state.apply(&settings);
        assert!(!update.refill, "the rows did not change");
        assert_eq!(update.selected, Some(1), "the user's row stays");
        // The player took the pick: it is the skin in use now.
        let taken = skin_settings(&["a", "b"], Some(1));
        let update = state.apply(&taken);
        assert_eq!(update.selected, Some(1), "the picked skin is selected");
        // A later push that names another skin wins over the pick.
        let other = skin_settings(&["a", "b"], Some(0));
        let update = state.apply(&other);
        assert_eq!(update.selected, Some(0), "the player wins");
    }

    #[test]
    fn a_pick_survives_a_change_of_the_rows() {
        let settings = skin_settings(&["a", "b"], Some(0));
        let mut state = SkinListState::default();
        state.apply(&settings);
        // The user picks the second skin; the player has not got the pick
        // yet.
        state.pick(1);
        // A new skin appears in the folder before the pick: the picked skin
        // is found in the new rows.
        let more = skin_settings(&["a", "b", "c"], Some(0));
        let update = state.apply(&more);
        assert!(update.refill, "the rows changed");
        assert_eq!(update.selected, Some(1), "the picked skin stays selected");
    }

    #[test]
    fn a_list_that_showed_nothing_is_not_the_skins_page_s_list() {
        let settings = skin_settings(&["a", "b"], Some(0));
        let empty = SkinListState::default();
        assert!(
            !empty.shows(&SkinListState::of(&settings)),
            "a list that showed nothing is not the list of the Skins page"
        );
        let mut state = SkinListState::default();
        state.apply(&settings);
        assert!(
            state.shows(&SkinListState::of(&settings)),
            "the list shows the rows of the Skins page"
        );
        assert!(
            !state.shows(&SkinListState::of(&skin_settings(&["a"], Some(0)))),
            "other rows are not the rows of the Skins page"
        );
    }

    #[test]
    fn settings_read_back_through_their_binding() {
        let settings = PrefsSettings {
            always_on_top: true,
            repeat: true,
            ..PrefsSettings::default()
        };
        assert!(settings.get(Bind::AlwaysOnTop));
        assert!(settings.get(Bind::Repeat));
        assert!(!settings.get(Bind::EasyMove));
        assert!(!settings.get(Bind::DoubleSize));
        assert!(!settings.get(Bind::Shuffle));
    }

    #[test]
    fn the_library_page_has_a_root_list_and_the_two_buttons() {
        let items = page_items(PrefsNode::Library);
        let count = |f: fn(&Item) -> bool| items.iter().filter(|i| f(i)).count();
        assert_eq!(count(|i| matches!(i, Item::LibraryRoots { .. })), 1);
        assert_eq!(count(|i| matches!(i, Item::LibraryAdd { .. })), 1);
        assert_eq!(count(|i| matches!(i, Item::LibraryRemove { .. })), 1);
        // The list is above the buttons: the user picks a root and then
        // removes it.
        let roots = items
            .iter()
            .find_map(|item| match item {
                Item::LibraryRoots { rect } => Some(*rect),
                _ => None,
            })
            .expect("the root list");
        let add = items
            .iter()
            .find_map(|item| match item {
                Item::LibraryAdd { rect } => Some(*rect),
                _ => None,
            })
            .expect("the add button");
        let remove = items
            .iter()
            .find_map(|item| match item {
                Item::LibraryRemove { rect } => Some(*rect),
                _ => None,
            })
            .expect("the remove button");
        assert!(roots.y + roots.h <= add.y, "the list is above the buttons");
        assert!(add.y + add.h <= remove.y + remove.h);
    }

    #[test]
    fn the_library_page_lists_the_roots_the_player_pushed_in() {
        let settings = PrefsSettings {
            library_roots: vec![
                LibraryRootRow {
                    path: r"C:\Music".to_string(),
                    last_scanned_at: None,
                },
                LibraryRootRow {
                    path: r"D:\Tunes".to_string(),
                    last_scanned_at: None,
                },
            ],
            ..PrefsSettings::default()
        };
        assert_eq!(settings.library_roots.len(), 2);
        assert_eq!(settings.library_root(0), Some(r"C:\Music"));
        assert_eq!(settings.library_root(1), Some(r"D:\Tunes"));
    }

    #[test]
    fn a_root_that_was_never_scanned_says_so() {
        assert_eq!(
            library_root_text(r"C:\Music", None),
            r"C:\Music — never scanned"
        );
    }

    #[test]
    fn a_root_names_the_utc_minute_of_its_last_scan() {
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(
            library_root_text(r"C:\Music", Some(1_700_000_000)),
            r"C:\Music — last scanned 2023-11-14 22:13 UTC"
        );
        // The epoch itself.
        assert_eq!(
            library_root_text(r"C:\Music", Some(0)),
            r"C:\Music — last scanned 1970-01-01 00:00 UTC"
        );
    }

    #[test]
    fn the_row_text_carries_the_path_and_the_time() {
        let row = LibraryRootRow {
            path: r"D:\Tunes".to_string(),
            last_scanned_at: Some(1_700_000_000),
        };
        let text = library_root_text(&row.path, row.last_scanned_at);
        assert!(text.starts_with(r"D:\Tunes"), "{text}");
        assert!(text.contains("last scanned"), "{text}");
        assert!(text.contains("2023-11-14 22:13 UTC"), "{text}");
    }

    #[test]
    fn the_library_actions_say_what_the_user_wanted() {
        let settings = PrefsSettings {
            library_roots: vec![
                LibraryRootRow {
                    path: r"C:\Music".to_string(),
                    last_scanned_at: None,
                },
                LibraryRootRow {
                    path: r"D:\Tunes".to_string(),
                    last_scanned_at: None,
                },
            ],
            ..PrefsSettings::default()
        };
        // The row the user picked names the root; a row that does not exist
        // names nothing.
        assert_eq!(settings.library_root(0), Some(r"C:\Music"));
        assert_eq!(settings.library_root(1), Some(r"D:\Tunes"));
        assert_eq!(settings.library_root(2), None);
        // The add button asks for a folder; the player picks it.
        assert_eq!(
            PrefsAction::AddLibraryRoot,
            PrefsAction::AddLibraryRoot,
            "the action carries no row: the folder is not chosen yet"
        );
        assert_ne!(
            PrefsAction::RemoveLibraryRoot(0),
            PrefsAction::RemoveLibraryRoot(1)
        );
    }
}
