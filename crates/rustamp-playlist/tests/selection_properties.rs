//! Property tests for the playlist editor's selection and editing model
//! (spec **S06** §2/§5, **S03** §2).
//!
//! The model under test is `rustamp_playlist`'s `interact` module — the plain
//! click, the shift-range, the ctrl-click toggle, the block move and the two
//! removals — on top of the core `Playlist`. The rules are the ones the
//! module states: a plain click replaces the selection with one row (S06 §2,
//! settled), a shift-click selects the clamped range between the anchor and
//! the target, a ctrl-click toggles exactly one row, a move keeps the
//! selected entries and only reorders them, `Delete` removes the selection
//! (S06 §5) and `Ctrl+Delete` crops to it (S03 §2, `1035`).
//!
//! The scenarios are driven by the crate's own [`Rng`]: no new dependency is
//! needed, and every case is reproducible from the printed seed and step.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rustamp_core::playlist::{EntryId, MediaLocation, Playlist};
use rustamp_playlist::{
    LoadedEntry, Rng, click_select, crop_to_selected, insert_loaded, move_selected,
    move_selected_down, move_selected_up, range_ids, remove_selected, select_range,
    toggle_select_at,
};

/// Seed of the random scenario stream.
const SEED: u64 = 0x5EED_2580_0000_0001;

/// Number of independent random scenarios per property.
const ROUNDS: usize = 800;

/// Number of operations per random scenario.
const STEPS: usize = 60;

/// The largest row a scenario may name.
///
/// The playlists hold fewer entries than this, so most of the clicks land
/// outside the list: the model has to refuse them, not panic.
const ROW_BOUND: usize = 12;

/// The editor's caller: the core playlist plus the shift-range anchor.
///
/// The anchor is **caller-owned** — the window keeps it (card S06i4 §3) and
/// hands its row to [`select_range`]. The crate itself keeps no anchor
/// state, so the harness holds the anchor as the *entry* of the last plain
/// click and re-derives its row with [`Playlist::index_of`], exactly as the
/// crate's docs prescribe. An anchor whose entry is removed dies with it.
#[derive(Debug, Clone)]
struct Editor {
    playlist: Playlist,
    anchor: Option<EntryId>,
}

impl Editor {
    /// An editor over a playlist of `count` entries.
    fn with_entries(count: usize) -> Self {
        let mut playlist = Playlist::new();
        for i in 0..count {
            playlist.push(MediaLocation::File(PathBuf::from(format!("t{i}.mp3"))));
        }
        Self {
            playlist,
            anchor: None,
        }
    }

    /// The entry ids, in playlist order.
    fn ids(&self) -> Vec<EntryId> {
        self.playlist.iter().map(|entry| entry.id).collect()
    }

    /// The selected entry ids, in playlist order.
    fn selected(&self) -> Vec<EntryId> {
        self.playlist.selected_in_order()
    }

    /// The selected entry ids, as a set.
    fn selected_set(&self) -> BTreeSet<EntryId> {
        self.playlist.selection().iter().copied().collect()
    }

    /// The row the anchor sits on, if the anchor still has one.
    fn anchor_row(&self) -> Option<usize> {
        self.anchor.and_then(|id| self.playlist.index_of(id))
    }

    /// A plain click: one row selected, and the clicked row is the new anchor.
    fn click(&mut self, index: usize) -> bool {
        let answered = click_select(&mut self.playlist, index);
        if answered {
            self.anchor = self.playlist.get_index(index).map(|entry| entry.id);
        }
        answered
    }

    /// A ctrl-click: one row toggled, the rest of the selection untouched.
    fn ctrl_click(&mut self, index: usize) -> bool {
        toggle_select_at(&mut self.playlist, index)
    }

    /// A shift-click from the anchor. Without a live anchor there is no range.
    fn shift_click(&mut self, target: usize) -> bool {
        let Some(row) = self.anchor_row() else {
            return false;
        };
        select_range(&mut self.playlist, row, target)
    }

    /// `Delete`: remove the selection, then re-derive the anchor.
    fn delete(&mut self) -> Vec<EntryId> {
        let removed = remove_selected(&mut self.playlist);
        self.rederive_anchor();
        removed
    }

    /// `Ctrl+Delete`: crop to the selection, then re-derive the anchor.
    fn crop(&mut self) -> Vec<EntryId> {
        let removed = crop_to_selected(&mut self.playlist);
        self.rederive_anchor();
        removed
    }

    /// The anchor rule after the list changed: the anchor is the *entry* of
    /// the last plain click, so it survives a removal that spared it and dies
    /// with one that took it. An empty list has no anchor at all.
    fn rederive_anchor(&mut self) {
        self.anchor = self
            .anchor
            .filter(|id| self.playlist.index_of(*id).is_some());
    }
}

/// The operations a scenario may apply to the editor.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// A plain click on a row.
    Click(usize),
    /// A shift-click from an arbitrary anchor row to an arbitrary target row.
    Shift(usize, usize),
    /// A shift-click from the anchor the editor kept.
    ShiftFromAnchor(usize),
    /// A ctrl-click on a row.
    Ctrl(usize),
    /// A drag that moves the selected block.
    MoveTo(usize),
    /// A move of a single entry to a row.
    MoveEntry(usize, usize),
    /// `Alt+Up`.
    MoveUp,
    /// `Alt+Down`.
    MoveDown,
    /// `Delete`.
    Delete,
    /// `Ctrl+Delete`.
    Crop,
    /// Insert an entry at a row.
    Insert(usize),
    /// Remove the entry at a row.
    RemoveAt(usize),
    /// `Ctrl+A`.
    SelectAll,
    /// `Ctrl+I`.
    Invert,
    /// Make a row the playing entry.
    SetCurrent(usize),
}

/// A random operation list.
///
/// The row indices come from a fixed bound that is larger than the playlists,
/// so most of them land outside the list and the model has to refuse them.
fn random_ops(rng: &mut Rng, count: usize) -> Vec<Op> {
    (0..count)
        .map(|_| match rng.below(15) {
            0 => Op::Click(rng.below(ROW_BOUND)),
            1 => Op::Shift(rng.below(ROW_BOUND), rng.below(ROW_BOUND)),
            2 => Op::ShiftFromAnchor(rng.below(ROW_BOUND)),
            3 => Op::Ctrl(rng.below(ROW_BOUND)),
            4 => Op::MoveTo(rng.below(ROW_BOUND)),
            5 => Op::MoveEntry(rng.below(ROW_BOUND), rng.below(ROW_BOUND)),
            6 => Op::MoveUp,
            7 => Op::MoveDown,
            8 => Op::Delete,
            9 => Op::Crop,
            10 => Op::Insert(rng.below(ROW_BOUND)),
            11 => Op::RemoveAt(rng.below(ROW_BOUND)),
            12 => Op::SelectAll,
            13 => Op::Invert,
            _ => Op::SetCurrent(rng.below(ROW_BOUND)),
        })
        .collect()
}

/// The rows a shift-click from `anchor` to `target` selects: both ends
/// clamped to the last row, the range between them inclusive.
fn expected_range(ids: &[EntryId], anchor: usize, target: usize) -> Vec<EntryId> {
    if ids.is_empty() {
        return Vec::new();
    }
    let last = ids.len() - 1;
    let a = anchor.min(last);
    let b = target.min(last);
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    ids[lo..=hi].to_vec()
}

/// The ids of `ids`, sorted, so two orders can be compared.
fn sorted(ids: &[EntryId]) -> Vec<EntryId> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids
}

/// The invariants that hold after every operation.
fn assert_invariants(editor: &Editor, what: &str) {
    let playlist = &editor.playlist;
    let live: BTreeSet<EntryId> = playlist.iter().map(|entry| entry.id).collect();

    // The selection is a subset of the entries: no id survives a removal it
    // does not belong to.
    for id in playlist.selection() {
        assert!(
            live.contains(id),
            "{what}: the selection holds the dead id {id:?}"
        );
    }
    assert_eq!(
        playlist.selection().len(),
        playlist.selected_in_order().len(),
        "{what}: the selection and the list disagree on how many rows are selected"
    );

    // The selected rows are in playlist order and without duplicates.
    let rows: Vec<usize> = playlist
        .selected_in_order()
        .iter()
        .map(|id| {
            playlist
                .index_of(*id)
                .expect("a selected id belongs to a live entry")
        })
        .collect();
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "{what}: the selected rows are not in playlist order: {rows:?}"
    );

    // The playing entry is a live entry or none.
    if let Some(current) = playlist.current() {
        assert!(
            live.contains(&current),
            "{what}: the dead entry {current:?} is still the playing entry"
        );
    }

    // The anchor is a live entry or none.
    if let Some(anchor) = editor.anchor {
        assert!(
            live.contains(&anchor),
            "{what}: the anchor points at the dead entry {anchor:?}"
        );
    }
}

/// Applies one operation and checks the properties that operation has to
/// hold.
fn apply_op(editor: &mut Editor, op: Op, what: &str) {
    match op {
        Op::Click(index) => {
            let answered = editor.click(index);
            if answered {
                let clicked = editor
                    .playlist
                    .get_index(index)
                    .expect("the clicked row exists");
                assert_eq!(
                    editor.selected(),
                    vec![clicked.id],
                    "{what}: a plain click must clear the previous selection and select one row"
                );
            }
        }
        Op::Shift(anchor, target) => {
            let len = editor.playlist.len();
            let before = editor.selected();
            let answered = select_range(&mut editor.playlist, anchor, target);
            if len == 0 {
                assert!(!answered, "{what}: an empty list must refuse the range");
                assert_eq!(
                    editor.selected(),
                    before,
                    "{what}: a refused range must change nothing"
                );
            } else {
                assert!(answered, "{what}: a range must be answered");
                let expected = expected_range(&editor.ids(), anchor, target);
                assert_eq!(
                    editor.selected(),
                    expected,
                    "{what}: the range must be the clamped [min, max] range"
                );
                assert_eq!(
                    range_ids(&editor.ids(), anchor, target),
                    expected,
                    "{what}: the window seam must agree with the model"
                );
            }
        }
        Op::ShiftFromAnchor(target) => {
            let before = editor.selected();
            let answered = editor.shift_click(target);
            match editor.anchor_row() {
                Some(row) => {
                    assert!(answered, "{what}: a range from a live anchor is answered");
                    assert_eq!(
                        editor.selected(),
                        expected_range(&editor.ids(), row, target),
                        "{what}: the range from the anchor must be the clamped range"
                    );
                }
                None => {
                    assert!(!answered, "{what}: a range without an anchor is refused");
                    assert_eq!(
                        editor.selected(),
                        before,
                        "{what}: a refused range must change nothing"
                    );
                }
            }
        }
        Op::Ctrl(index) => {
            let before = editor.selected_set();
            let answered = editor.ctrl_click(index);
            if answered {
                let toggled = editor
                    .playlist
                    .get_index(index)
                    .expect("the toggled row exists")
                    .id;
                let mut expected = before.clone();
                if !expected.remove(&toggled) {
                    expected.insert(toggled);
                }
                assert_eq!(
                    editor.selected_set(),
                    expected,
                    "{what}: a ctrl-click must toggle exactly one row"
                );
            } else {
                assert_eq!(
                    editor.selected_set(),
                    before,
                    "{what}: a ctrl-click outside the list must change nothing"
                );
            }
        }
        Op::MoveTo(to) => {
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            let before_current = editor.playlist.current();
            let answered = move_selected(&mut editor.playlist, to);
            assert_move(
                editor,
                &before_ids,
                &before_selected,
                before_current,
                answered,
                what,
            );
        }
        Op::MoveEntry(row, to) => {
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            let before_current = editor.playlist.current();
            let Some(entry) = editor.playlist.get_index(row).cloned() else {
                return;
            };
            assert!(editor.playlist.move_entry(entry.id, to));
            let after_ids = editor.ids();
            assert_eq!(
                after_ids.len(),
                before_ids.len(),
                "{what}: a single move must not change the number of entries"
            );
            assert_eq!(
                sorted(&after_ids),
                sorted(&before_ids),
                "{what}: a single move must be a permutation of the same entries"
            );
            assert_eq!(
                editor.selected_set(),
                before_selected,
                "{what}: a single move must keep the selected entries"
            );
            assert_eq!(
                editor.playlist.current(),
                before_current,
                "{what}: a single move must keep the playing entry"
            );
        }
        Op::MoveUp => {
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            let before_current = editor.playlist.current();
            let answered = move_selected_up(&mut editor.playlist);
            assert_move(
                editor,
                &before_ids,
                &before_selected,
                before_current,
                answered,
                what,
            );
        }
        Op::MoveDown => {
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            let before_current = editor.playlist.current();
            let answered = move_selected_down(&mut editor.playlist);
            assert_move(
                editor,
                &before_ids,
                &before_selected,
                before_current,
                answered,
                what,
            );
        }
        Op::Delete => {
            let before_ids = editor.ids();
            let before_selected = editor.selected();
            let before_current = editor.playlist.current();
            let removed = editor.delete();
            assert_eq!(
                removed, before_selected,
                "{what}: Delete must remove the selection in playlist order"
            );
            assert!(
                editor.selected().is_empty(),
                "{what}: the selection must be empty after the removal"
            );
            let after_ids = editor.ids();
            assert_eq!(
                after_ids.len() + removed.len(),
                before_ids.len(),
                "{what}: Delete must remove exactly the selected entries"
            );
            for id in &after_ids {
                assert!(
                    before_ids.contains(id),
                    "{what}: the removal invented the entry {id:?}"
                );
            }
            for id in &removed {
                assert!(
                    !after_ids.contains(id),
                    "{what}: the removed entry {id:?} is still in the list"
                );
            }
            assert_current(editor, &before_ids, before_current, &removed, what);
        }
        Op::Crop => {
            let before_ids = editor.ids();
            let before_selected = editor.selected();
            let before_current = editor.playlist.current();
            let removed = editor.crop();
            if before_selected.is_empty() {
                assert!(
                    removed.is_empty(),
                    "{what}: an unstaged crop must be a no-op"
                );
            } else {
                assert_eq!(
                    editor.ids(),
                    before_selected,
                    "{what}: a crop must keep exactly the selection"
                );
                assert_eq!(
                    editor.selected(),
                    before_selected,
                    "{what}: a crop must keep the selection"
                );
            }
            assert_current(editor, &before_ids, before_current, &removed, what);
        }
        Op::Insert(index) => {
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            let before_current = editor.playlist.current();
            let id = editor
                .playlist
                .insert(index, MediaLocation::File(PathBuf::from("new.mp3")));
            assert!(
                before_ids.iter().all(|old| *old < id),
                "{what}: an entry id must never be reused"
            );
            assert_eq!(
                editor.ids().len(),
                before_ids.len() + 1,
                "{what}: the insertion must add exactly one entry"
            );
            assert_eq!(
                editor.selected_set(),
                before_selected,
                "{what}: an insertion must keep the selected entries"
            );
            assert_eq!(
                editor.playlist.current(),
                before_current,
                "{what}: an insertion must keep the playing entry"
            );
        }
        Op::RemoveAt(index) => {
            let Some(entry) = editor.playlist.get_index(index).cloned() else {
                return;
            };
            editor.playlist.remove(entry.id);
            assert!(
                !editor.playlist.is_selected(entry.id),
                "{what}: a removed entry must leave the selection"
            );
            editor.rederive_anchor();
        }
        Op::SelectAll => {
            editor.playlist.select_all();
            assert_eq!(
                editor.selected().len(),
                editor.playlist.len(),
                "{what}: select all must select every entry"
            );
        }
        Op::Invert => {
            let before = editor.selected_set();
            editor.playlist.invert_selection();
            let live: BTreeSet<EntryId> = editor.ids().into_iter().collect();
            assert_eq!(
                editor.selected_set(),
                live.difference(&before).copied().collect(),
                "{what}: the inverted selection must be the complement"
            );
        }
        Op::SetCurrent(index) => {
            let id = editor.playlist.get_index(index).map(|entry| entry.id);
            editor.playlist.set_current(id);
        }
    }
}

/// Checks the properties of a move: the entries are only reordered, the
/// selection and the playing entry stay, and the answer says whether the
/// order changed.
fn assert_move(
    editor: &Editor,
    before_ids: &[EntryId],
    before_selected: &BTreeSet<EntryId>,
    before_current: Option<EntryId>,
    answered: bool,
    what: &str,
) {
    let after_ids = editor.ids();
    assert_eq!(
        after_ids.len(),
        before_ids.len(),
        "{what}: a move must not change the number of entries"
    );
    assert_eq!(
        sorted(&after_ids),
        sorted(before_ids),
        "{what}: a move must be a permutation of the same entries"
    );
    assert_eq!(
        &editor.selected_set(),
        before_selected,
        "{what}: a move must keep the selected entries"
    );
    assert_eq!(
        editor.playlist.current(),
        before_current,
        "{what}: a move must keep the playing entry"
    );
    assert_eq!(
        answered,
        before_ids != after_ids.as_slice(),
        "{what}: the move must report whether the order changed"
    );
}

/// Checks the properties of a removal for the playing entry: it survives with
/// its row shifted by the removed rows before it, or it is cleared.
fn assert_current(
    editor: &Editor,
    before_ids: &[EntryId],
    before_current: Option<EntryId>,
    removed: &[EntryId],
    what: &str,
) {
    match before_current {
        None => assert_eq!(
            editor.playlist.current(),
            None,
            "{what}: the removal must not invent a playing entry"
        ),
        Some(current) if removed.contains(&current) => assert_eq!(
            editor.playlist.current(),
            None,
            "{what}: the removed playing entry must be cleared"
        ),
        Some(current) => {
            assert_eq!(
                editor.playlist.current(),
                Some(current),
                "{what}: the playing entry must survive the removal"
            );
            let old_row = before_ids
                .iter()
                .position(|id| *id == current)
                .expect("the playing entry was in the list");
            let new_row = editor
                .ids()
                .iter()
                .position(|id| *id == current)
                .expect("the playing entry is still in the list");
            let removed_before = removed
                .iter()
                .filter(|id| {
                    before_ids
                        .iter()
                        .position(|other| other == *id)
                        .expect("a removed id was in the list")
                        < old_row
                })
                .count();
            assert_eq!(
                new_row,
                old_row - removed_before,
                "{what}: the playing entry's row must shift by the removed rows before it"
            );
        }
    }
}

/// Every random sequence of editor gestures keeps the model consistent, and
/// no sequence panics.
#[test]
fn random_gesture_sequences_keep_the_model_consistent() {
    let mut rng = Rng::new(SEED);
    let mut saw_refused_click = 0usize;
    let mut saw_range = 0usize;
    let mut saw_toggle_off = 0usize;
    let mut saw_move = 0usize;
    let mut saw_delete = 0usize;
    let mut saw_crop = 0usize;
    let mut saw_stale_anchor = 0usize;
    let mut saw_empty = 0usize;
    for round in 0..ROUNDS {
        let count = rng.below(9);
        let mut editor = Editor::with_entries(count);
        let ops = random_ops(&mut rng, STEPS);
        for (step, op) in ops.iter().copied().enumerate() {
            let what = format!(
                "round {round}, step {step}, op {op:?}, {} entries",
                editor.playlist.len()
            );
            let len = editor.playlist.len();
            let before_ids = editor.ids();
            let before_selected = editor.selected_set();
            apply_op(&mut editor, op, &what);

            // The generator has to reach the interesting states, otherwise
            // the properties below would hold trivially.
            match op {
                Op::Click(index) if index >= len => saw_refused_click += 1,
                Op::Shift(anchor, _) if anchor >= len => saw_stale_anchor += 1,
                Op::ShiftFromAnchor(_) if editor.anchor_row().is_none() => saw_stale_anchor += 1,
                Op::Shift(_, _) if editor.selected().len() > 1 => saw_range += 1,
                Op::Ctrl(_) if editor.selected_set() != before_selected => saw_toggle_off += 1,
                Op::MoveTo(_) | Op::MoveUp | Op::MoveDown | Op::MoveEntry(_, _) => {
                    if editor.ids() != before_ids {
                        saw_move += 1;
                    }
                }
                Op::Delete if editor.playlist.len() < len => saw_delete += 1,
                Op::Crop if editor.playlist.len() < len => saw_crop += 1,
                _ => {}
            }
            if editor.playlist.is_empty() {
                saw_empty += 1;
            }
            assert_invariants(&editor, &what);
        }
    }
    assert!(
        saw_refused_click > 0,
        "no scenario ever clicked outside the list"
    );
    assert!(saw_range > 0, "no scenario ever held a multi-row selection");
    assert!(saw_toggle_off > 0, "no scenario ever toggled a row");
    assert!(saw_move > 0, "no scenario ever moved the selection");
    assert!(saw_delete > 0, "no scenario ever deleted the selection");
    assert!(saw_crop > 0, "no scenario ever cropped to the selection");
    assert!(
        saw_stale_anchor > 0,
        "no scenario ever used an anchor outside the list"
    );
    assert!(saw_empty > 0, "no scenario ever reached an empty list");
}

/// The selection is always a subset of the entries: a removed entry never
/// stays selected, whatever else happened to the list.
#[test]
fn a_removed_entry_never_stays_selected() {
    let mut rng = Rng::new(SEED ^ 0x5E1E_C710);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        editor.playlist.select_all();
        let ids = editor.ids();
        let victim = ids[rng.below(ids.len())];
        editor.playlist.remove(victim);
        assert!(
            !editor.playlist.is_selected(victim),
            "round {round}: the removed entry is still selected"
        );
        let live: BTreeSet<EntryId> = editor.ids().into_iter().collect();
        for id in editor.playlist.selection() {
            assert!(
                live.contains(id),
                "round {round}: the selection holds the dead id {id:?}"
            );
        }
    }
}

/// A shift-range is the contiguous range between the anchor and the target,
/// with both ends clamped to the last row.
#[test]
fn a_shift_range_is_the_contiguous_clamped_range() {
    let mut rng = Rng::new(SEED ^ 0x5A4C_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        let anchor = rng.below(ROW_BOUND);
        let target = rng.below(ROW_BOUND);
        assert!(select_range(&mut editor.playlist, anchor, target));

        let ids = editor.ids();
        let expected = expected_range(&ids, anchor, target);
        assert_eq!(
            editor.selected(),
            expected,
            "round {round}: the range is not the clamped [min, max] range"
        );

        // The range is contiguous: the selected rows are consecutive.
        let rows: Vec<usize> = editor
            .selected()
            .iter()
            .map(|id| {
                editor
                    .playlist
                    .index_of(*id)
                    .expect("a selected id is live")
            })
            .collect();
        assert!(
            rows.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "round {round}: the range is not contiguous: {rows:?}"
        );

        // The window seam agrees with the model.
        assert_eq!(
            range_ids(&ids, anchor, target),
            editor.selected(),
            "round {round}: the window seam disagrees with the model"
        );
    }
}

/// A plain click clears the previous selection and selects exactly one row.
#[test]
fn a_plain_click_selects_exactly_one_row() {
    let mut rng = Rng::new(SEED ^ 0xC11C_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        // A wide selection first, so the click has something to clear.
        editor.playlist.select_all();
        let index = rng.below(count);
        assert!(editor.click(index));
        let clicked = editor
            .playlist
            .get_index(index)
            .expect("the clicked row exists")
            .id;
        assert_eq!(
            editor.selected(),
            vec![clicked],
            "round {round}: a plain click must select exactly the clicked row"
        );

        // A click outside the list selects nothing and clears nothing.
        let before = editor.selected();
        assert!(!editor.click(count + rng.below(4)));
        assert_eq!(
            editor.selected(),
            before,
            "round {round}: a click outside the list must change nothing"
        );
    }
}

/// A ctrl-click toggles exactly one row and never clears the others.
#[test]
fn a_ctrl_click_toggles_one_row_and_keeps_the_others() {
    let mut rng = Rng::new(SEED ^ 0xC721_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        editor.playlist.select_all();
        let index = rng.below(count);
        let toggled = editor
            .playlist
            .get_index(index)
            .expect("the toggled row exists")
            .id;
        let others: Vec<EntryId> = editor
            .selected()
            .into_iter()
            .filter(|id| *id != toggled)
            .collect();

        // The row leaves the selection, the others stay.
        assert!(editor.ctrl_click(index));
        assert!(
            !editor.playlist.is_selected(toggled),
            "round {round}: the toggled row is still selected"
        );
        for id in &others {
            assert!(
                editor.playlist.is_selected(*id),
                "round {round}: the ctrl-click cleared the other row {id:?}"
            );
        }

        // And it comes back.
        assert!(editor.ctrl_click(index));
        assert!(
            editor.playlist.is_selected(toggled),
            "round {round}: the toggled row did not come back"
        );

        // A ctrl-click outside the list changes nothing.
        let before = editor.selected_set();
        assert!(!editor.ctrl_click(count + rng.below(4)));
        assert_eq!(
            editor.selected_set(),
            before,
            "round {round}: a ctrl-click outside the list must change nothing"
        );
    }
}

/// A move keeps the selected entries and only reorders them, and a block
/// that moves into itself changes nothing.
#[test]
fn a_move_keeps_the_selection_and_moving_into_itself_is_a_no_op() {
    let mut rng = Rng::new(SEED ^ 0xB10C_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        let first = rng.below(count);
        let last = first + rng.below(count - first);
        let ids = editor.ids();
        editor.playlist.select_only(&ids[first..=last]);
        let selected = editor.selected_set();

        // The block's own position: the move is a no-op.
        let before = editor.ids();
        assert!(
            !move_selected(&mut editor.playlist, first),
            "round {round}: a block that moves into itself must report no change"
        );
        assert_eq!(
            editor.ids(),
            before,
            "round {round}: a block that moves into itself must not reorder"
        );
        assert_eq!(
            editor.selected_set(),
            selected,
            "round {round}: a no-op move must keep the selection"
        );

        // Every other position reorders the rows but keeps the entries.
        let to = rng.below(count);
        let before = editor.ids();
        let answered = move_selected(&mut editor.playlist, to);
        assert_eq!(
            answered,
            before != editor.ids(),
            "round {round}: the move must report whether the order changed"
        );
        assert_eq!(
            sorted(&editor.ids()),
            sorted(&before),
            "round {round}: a move must be a permutation of the same entries"
        );
        assert_eq!(
            editor.selected_set(),
            selected,
            "round {round}: a move must keep the selected entries"
        );
    }
}

/// A contiguous block moves exactly one row up or down, and the move is
/// refused at the ends of the list.
#[test]
fn a_contiguous_block_moves_exactly_one_row() {
    for count in 2..=8usize {
        for first in 0..count {
            for last in first..count {
                let mut editor = Editor::with_entries(count);
                let ids = editor.ids();
                editor.playlist.select_only(&ids[first..=last]);
                let before = editor.ids();

                // One row up: the block takes the place of the row above it.
                if first > 0 {
                    let mut expected = before[..first - 1].to_vec();
                    expected.extend_from_slice(&before[first..=last]);
                    expected.push(before[first - 1]);
                    expected.extend_from_slice(&before[last + 1..]);
                    assert!(
                        move_selected_up(&mut editor.playlist),
                        "count {count}, block {first}..={last}: the block must move up"
                    );
                    assert_eq!(
                        editor.ids(),
                        expected,
                        "count {count}, block {first}..={last}: the block must move one row up"
                    );
                } else {
                    assert!(
                        !move_selected_up(&mut editor.playlist),
                        "count {count}, block {first}..={last}: the block is already at the top"
                    );
                    assert_eq!(
                        editor.ids(),
                        before,
                        "count {count}, block {first}..={last}: a refused move changes nothing"
                    );
                }

                // One row down: the block takes the place of the row below
                // it, and that row moves up to the head of the block.
                let mut editor = Editor::with_entries(count);
                let ids = editor.ids();
                editor.playlist.select_only(&ids[first..=last]);
                let before = editor.ids();
                if last + 1 < count {
                    let mut expected = before[..first].to_vec();
                    expected.push(before[last + 1]);
                    expected.extend_from_slice(&before[first..=last]);
                    expected.extend_from_slice(&before[last + 2..]);
                    assert!(
                        move_selected_down(&mut editor.playlist),
                        "count {count}, block {first}..={last}: the block must move down"
                    );
                    assert_eq!(
                        editor.ids(),
                        expected,
                        "count {count}, block {first}..={last}: the block must move one row down"
                    );
                } else {
                    assert!(
                        !move_selected_down(&mut editor.playlist),
                        "count {count}, block {first}..={last}: the block is already at the bottom"
                    );
                    assert_eq!(
                        editor.ids(),
                        before,
                        "count {count}, block {first}..={last}: a refused move changes nothing"
                    );
                }
            }
        }
    }
}

/// A non-contiguous selection moves as one block: the entries are gathered,
/// they keep their relative order, and the block lands at the end of the
/// list the move heads for.
///
/// **Own choice** (S06 §8 Q6 leaves the drag semantics open): the crate moves
/// the whole selection as one block, so the gaps between the selected rows
/// close.
#[test]
fn a_non_contiguous_selection_moves_as_one_block() {
    let mut editor = Editor::with_entries(6);
    let ids = editor.ids();
    editor.playlist.select_only(&[ids[1], ids[4]]);
    let selected = editor.selected();

    // `Alt+Up` gathers the block and puts it at the top of the list.
    assert!(move_selected_up(&mut editor.playlist));
    assert_eq!(
        editor.ids(),
        vec![ids[1], ids[4], ids[0], ids[2], ids[3], ids[5]],
        "the gathered block must land at the top"
    );
    assert_eq!(
        editor.selected(),
        selected,
        "the move must keep the selected entries"
    );

    // `Alt+Down` gathers the block and puts it at the bottom.
    let mut editor = Editor::with_entries(6);
    let ids = editor.ids();
    editor.playlist.select_only(&[ids[1], ids[4]]);
    let selected = editor.selected();
    assert!(move_selected_down(&mut editor.playlist));
    assert_eq!(
        editor.ids(),
        vec![ids[0], ids[2], ids[3], ids[5], ids[1], ids[4]],
        "the gathered block must land at the bottom"
    );
    assert_eq!(
        editor.selected(),
        selected,
        "the move must keep the selected entries"
    );
}

/// A removal makes the shift-range anchor stale, and the crate clamps it
/// into the new range: the gesture never names a row outside the list, and
/// an empty list refuses it.
#[test]
fn a_stale_anchor_is_clamped_into_the_new_range() {
    let mut rng = Rng::new(SEED ^ 0x57A1_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        let anchor = rng.below(ROW_BOUND);
        let target = rng.below(ROW_BOUND);
        assert!(select_range(&mut editor.playlist, anchor, target));

        // The removal shortens the list, so the anchor is stale.
        let stale = anchor;
        editor.delete();
        let len = editor.playlist.len();
        if len == 0 {
            assert!(
                !select_range(&mut editor.playlist, stale, target),
                "round {round}: an empty list must refuse the range"
            );
            assert!(
                editor.selected().is_empty(),
                "round {round}: an empty list has nothing selected"
            );
        } else {
            assert!(
                select_range(&mut editor.playlist, stale, target),
                "round {round}: a range on a non-empty list must be answered"
            );
            assert_eq!(
                editor.selected(),
                expected_range(&editor.ids(), stale, target),
                "round {round}: the stale anchor must be clamped into the new range"
            );
            for id in editor.selected() {
                assert!(
                    editor.playlist.index_of(id).is_some(),
                    "round {round}: the range selected a row outside the list"
                );
            }
        }
    }
}

/// The anchor dies with its entry: after a removal that took the anchor's
/// entry there is no anchor, and the shift-click is refused.
#[test]
fn the_anchor_dies_with_its_entry() {
    let mut rng = Rng::new(SEED ^ 0xDEAD_0001);
    for round in 0..ROUNDS {
        let count = 2 + rng.below(6);
        let mut editor = Editor::with_entries(count);
        let index = rng.below(count);
        assert!(editor.click(index));
        let anchor = editor.anchor.expect("the click set the anchor");
        editor.playlist.remove(anchor);
        editor.rederive_anchor();
        assert!(
            editor.anchor.is_none(),
            "round {round}: the anchor outlived its entry"
        );
        assert!(editor.anchor_row().is_none());
        let before = editor.selected();
        assert!(
            !editor.shift_click(rng.below(ROW_BOUND)),
            "round {round}: a shift-click without an anchor must be refused"
        );
        assert_eq!(
            editor.selected(),
            before,
            "round {round}: a refused shift-click must change nothing"
        );
    }
}

/// Loading entries into the list keeps the selection and the playing entry:
/// the ids are stable, so the selected rows only move.
#[test]
fn loading_entries_keeps_the_selection_and_the_playing_entry() {
    let mut rng = Rng::new(SEED ^ 0x10AD_0001);
    for round in 0..ROUNDS {
        let count = 1 + rng.below(8);
        let mut editor = Editor::with_entries(count);
        editor.playlist.select_all();
        let current = editor.ids()[0];
        editor.playlist.set_current(Some(current));
        let before_selected = editor.selected_set();
        let before_current = editor.playlist.current();

        let index = rng.below(ROW_BOUND);
        let entries = vec![
            LoadedEntry::new(MediaLocation::File(PathBuf::from("loaded.mp3")))
                .with_title("Loaded")
                .with_duration_ms(1234),
        ];
        let ids = insert_loaded(&mut editor.playlist, index, &entries);
        assert_eq!(ids.len(), 1, "round {round}: one entry was loaded");
        assert_eq!(
            editor.selected_set(),
            before_selected,
            "round {round}: the load must keep the selected entries"
        );
        assert_eq!(
            editor.playlist.current(),
            before_current,
            "round {round}: the load must keep the playing entry"
        );
        let loaded = editor
            .playlist
            .get(ids[0])
            .expect("the loaded entry is in the list");
        assert_eq!(loaded.title.as_deref(), Some("Loaded"));
        assert_eq!(loaded.duration_ms, Some(1234));
    }
}

/// No sequence of gestures panics — not on an empty playlist, not on a row
/// outside the list and not with a stale anchor.
#[test]
fn no_short_gesture_sequence_panics() {
    let ops = [
        Op::Click(0),
        Op::Click(2),
        Op::Shift(0, 3),
        Op::ShiftFromAnchor(1),
        Op::Ctrl(1),
        Op::MoveTo(0),
        Op::MoveEntry(0, 1),
        Op::MoveUp,
        Op::MoveDown,
        Op::Delete,
        Op::Crop,
        Op::Insert(1),
        Op::RemoveAt(1),
        Op::SelectAll,
        Op::Invert,
        Op::SetCurrent(0),
    ];
    for count in 0..=3usize {
        let mut editor = Editor::with_entries(count);
        explore(&mut editor, &ops, 3);
    }
}

/// Applies every operation sequence of the given depth and checks the
/// invariants after every step.
fn explore(editor: &mut Editor, ops: &[Op], depth: usize) {
    if depth == 0 {
        return;
    }
    for op in ops {
        let mut next = editor.clone();
        let what = format!("exhaustive scenario, op {op:?}");
        apply_op(&mut next, *op, &what);
        assert_invariants(&next, &what);
        explore(&mut next, ops, depth - 1);
    }
}
