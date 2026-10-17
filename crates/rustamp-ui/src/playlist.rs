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

use std::collections::BTreeSet;

use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
use rustamp_core::playlist::EntryId;

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

/// The playlist editor's headless behaviour: list view, selection and the
/// S06-settled gestures.
///
/// The machine tracks entry order as [`EntryId`]s (synced from the app via
/// [`PlaylistWindow::set_entries`]); selection is a set of ids. The visible
/// slice is [`VISIBLE_ROWS`] entries starting at the scroll offset.
#[derive(Debug, Clone, Default)]
pub struct PlaylistWindow {
    entries: Vec<EntryId>,
    first_visible: usize,
    selected: BTreeSet<EntryId>,
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
        self.entries = entries;
        let live: BTreeSet<EntryId> = self.entries.iter().copied().collect();
        self.selected.retain(|id| live.contains(id));
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

    /// A single click on the list selects the first visible row (S06 §2,
    /// settled: 2918 px, bbox (12,23)-(254,93), reproduced three times).
    ///
    /// Row order is unchanged (S06 §2, visual reading); on an empty list this
    /// is a no-op.
    pub fn click_list_first_visible(&mut self) -> Option<Command> {
        let first = self.visible_ids().into_iter().next()?;
        self.selected.clear();
        self.selected.insert(first);
        Some(Command::Select(SelectOp::Set(vec![first])))
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
    pub fn select_all(&mut self) -> Command {
        self.apply_select(&SelectOp::All);
        Command::Select(SelectOp::All)
    }

    /// Ctrl+I inverts the selection (S03 §2, `40171` "Invert selection",
    /// menu-confirmed; effect not measured).
    pub fn invert_selection(&mut self) -> Command {
        self.apply_select(&SelectOp::Invert);
        Command::Select(SelectOp::Invert)
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
    fn a_single_click_selects_the_first_visible_row() {
        // S06 §2: single posted click at (60,28) selects the first visible row.
        let mut window = window_with(60);
        window.scroll_to(56);
        let command = window.click_list_first_visible();
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
        assert_eq!(window.click_list_first_visible(), None);
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
        window.click_list_first_visible();
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
        let mut window = window_with(6);
        assert_eq!(window.select_all(), Command::Select(SelectOp::All));
        assert_eq!(window.selected_ids(), ids(6));
        window.apply_select(&SelectOp::Set(vec![EntryId(1), EntryId(3)]));
        assert_eq!(window.invert_selection(), Command::Select(SelectOp::Invert));
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
}
