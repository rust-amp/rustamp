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

use crate::shortcuts::{Key, Modifiers};

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
}

/// Maps a key press to the playlist editor's key action, if any.
///
/// Covers the S03 §2 rows listed on [`PlaylistKeyAction`]: plain Up, Down,
/// Home, End, PageUp, PageDown and Enter with no modifiers; `Alt+Up` and
/// `Alt+Down` for moving the selected block; `Alt+Delete` for removing dead
/// files; and `Ctrl+Delete` for cropping to the selection. The `Ctrl+Shift`
/// rows are tested first because [`Modifiers`] is a bit set that no
/// single-modifier match can express: `Ctrl+Shift+Delete` and `Ctrl+N` clear
/// the playlist (S03 §2 binds the one command id `40214` to both chords) and
/// `Ctrl+Shift+1/2/3` / `Ctrl+Shift+R` are the three sorts and randomize.
/// Plain `Delete`, `Ctrl+A` and `Ctrl+I` return `None` here to preserve the
/// existing app-side shortcut routing (`App::playlist_key_down`).
///
/// The app consults the map in `App::playlist_key_down`; this function only
/// names the row, it never runs it.
pub fn playlist_key_action(key: Key, modifiers: Modifiers) -> Option<PlaylistKeyAction> {
    // Checked before the single-modifier arms: `Modifiers` is a set, so
    // Ctrl+Shift is neither `Modifiers::CTRL` nor `Modifiers::SHIFT`.
    if modifiers == Modifiers::CTRL.union(Modifiers::SHIFT) {
        return match key {
            Key::Delete => Some(PlaylistKeyAction::ClearPlaylist),
            Key::Digit(1) => Some(PlaylistKeyAction::SortByTitle),
            Key::Digit(2) => Some(PlaylistKeyAction::SortByFileName),
            Key::Digit(3) => Some(PlaylistKeyAction::SortByPathAndFileName),
            Key::Letter(b'R') => Some(PlaylistKeyAction::Randomize),
            _ => None,
        };
    }
    match modifiers {
        Modifiers::NONE => match key {
            Key::Up => Some(PlaylistKeyAction::MoveSelectionUp),
            Key::Down => Some(PlaylistKeyAction::MoveSelectionDown),
            Key::Home => Some(PlaylistKeyAction::JumpToFirst),
            Key::End => Some(PlaylistKeyAction::JumpToLast),
            Key::PageUp => Some(PlaylistKeyAction::PageUp),
            Key::PageDown => Some(PlaylistKeyAction::PageDown),
            Key::Enter => Some(PlaylistKeyAction::PlaySelected),
            _ => None,
        },
        Modifiers::ALT => match key {
            Key::Up => Some(PlaylistKeyAction::MoveBlockUp),
            Key::Down => Some(PlaylistKeyAction::MoveBlockDown),
            Key::Delete => Some(PlaylistKeyAction::RemoveDead),
            _ => None,
        },
        Modifiers::CTRL => match key {
            Key::Delete => Some(PlaylistKeyAction::Crop),
            Key::Letter(b'N') => Some(PlaylistKeyAction::ClearPlaylist),
            _ => None,
        },
        _ => None,
    }
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
        // Modifiers take the shared path; the existing Delete/Ctrl+A/Ctrl+I
        // app path is untouched by this map.
        assert_eq!(playlist_key_action(Key::Up, Modifiers::CTRL), None);
        assert_eq!(playlist_key_action(Key::Enter, Modifiers::CTRL), None);
        assert_eq!(
            playlist_key_action(Key::Letter(b'A'), Modifiers::CTRL),
            None
        );
        assert_eq!(playlist_key_action(Key::Delete, Modifiers::NONE), None);
        assert_eq!(playlist_key_action(Key::Left, Modifiers::NONE), None);

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
        // Ctrl+Delete keeps cropping; plain Delete stays app-side.
        assert_eq!(
            playlist_key_action(Key::Delete, Modifiers::CTRL),
            Some(PlaylistKeyAction::Crop)
        );
        assert_eq!(playlist_key_action(Key::Delete, Modifiers::NONE), None);
    }
}
