//! Playlist-editor model interactions (spec **S06** §2/§5 and §8; **S03** §2
//! and §5).
//!
//! The core [`Playlist`](rustamp_core::playlist::Playlist) owns the entries,
//! their stable [`EntryId`]s, the selection set and the current entry. What it
//! has no operation for is the *editor's* gestures: a contiguous shift-click
//! range, a ctrl-click toggle, a drag that moves a block of selected rows, and
//! a delete that removes them. This module adds exactly those, as plain
//! functions over a `&mut Playlist`, so any caller that owns a core playlist
//! (the app, the playlist-window machine, a future engine-side model) can reach
//! them.
//!
//! ## What the spec settles, and what it does not
//!
//! Each rule below cites the spec section and says plainly whether it is
//! **settled** or this crate's **own choice**.
//!
//! * S06 §2 settles a single click (selects the first visible row; 2918 px) and
//!   SEL (selects all visible rows; 12203 px). The single-click selection set —
//!   one row, replacing everything before it — is therefore **settled**, which
//!   is what [`click_select`] implements. **Shift-click, ctrl-click and any
//!   contiguous range are not settled**: S06 §8 records that beyond the
//!   all-select of §2 "nothing about SEL visuals or multi-selection is settled"
//!   (Q11, Q14 remainder). The range and toggle behaviour here is therefore an
//!   **own choice**: the ordinary desktop meaning (shift extends a contiguous
//!   range from an anchor, ctrl toggles one row), chosen because the two settled
//!   gestures are a single row and the whole list, and a selection that can be
//!   neither extended nor trimmed would make the settled `Remove the selected
//!   item(s)` (S03 §2, `1034`) unusable past one row.
//! * S03 §2 lists `Delete` (`1034`, "Remove the selected item(s)") and
//!   `Ctrl+Delete` (`1035`, "Crop to the selected item(s)") as menu-confirmed
//!   rows, so *that* the selection is deleted is **spec-supported**. **What
//!   happens to the playing entry and the current index is not settled**; this
//!   module follows the core [`Playlist::remove`] contract: removing the current
//!   entry clears it, and every surviving entry keeps its id, so its index
//!   shifts down by exactly the number of removed entries before it.
//! * S03 §5 lists "Drag in the playlist | reorder / move entries" as
//!   **provisional**, and S06 §8 Q6 could not decide "reorder vs selection
//!   block" from the captures. This module moves the selected **block**, with
//!   the core [`Playlist::move_entries`] convention, which matches both
//!   provisional readings at once.
//! * S03 §2 `Alt+Up`/`Alt+Down` (`40215`/`40216`, "Move the selected entry
//!   up/down") are **provisional** rows with no measured effect; they are the
//!   keyboard face of the same block move.
//!
//! * **The shift-range anchor belongs to the caller.** This crate keeps no
//!   anchor state: the caller keeps the *entry* of the last plain click and
//!   hands its row to [`select_range`], which clamps the row into the list.
//!   A stale row — one a removal or a reorder left behind — therefore names a
//!   row inside the list or is refused, never a row that is not there. The
//!   window half owns the anchor, because it owns the row the user clicked.
//!
//! Entry ids are stable and never reused, so a reorder never changes *which*
//! entry is current: only its index moves with its row. That is the core's
//! guarantee and the reason the current entry "keeps playing" through a drag.

use rustamp_core::playlist::{EntryId, Playlist};

/// Plain click: replace the selection with the single entry at `index`.
///
/// The **settled** half of S06 §2 (a single click selects one row — 2918 px,
/// reproduced three times) restated for the model: the gesture's *pixel point*
/// is settled as "the first visible row", but what it means to the selection
/// set is a single-entry replacement, which is exactly this. Returns `false`
/// when `index` is past the end; an empty playlist therefore returns `false`
/// for every index.
///
/// Own choice within the settled rule: the clicked entry is the new
/// shift-range anchor. The crate keeps no anchor state, so the caller sets
/// its anchor to the clicked entry; the range rows are not settled (§8 Q14
/// remainder) and any anchor rule must be stated by the implementer.
pub fn click_select(playlist: &mut Playlist, index: usize) -> bool {
    let Some(id) = playlist.get_index(index).map(|entry| entry.id) else {
        return false;
    };
    playlist.select_only(&[id]);
    true
}

/// Shift-click: replace the selection with the contiguous range from `anchor`
/// to `target` (inclusive).
///
/// Both indices are clamped to the last row, so a click below the list extends
/// to the end; the order of the two arguments does not matter. Returns `false`
/// (and changes nothing) on an empty playlist.
///
/// **Own choice** (the spec does not settle multi-selection; see the module
/// docs): the ordinary desktop shift-click meaning, a *contiguous* range that
/// **replaces** the previous selection.
///
/// **The anchor rule (own choice).** The anchor is the *entry* of the last
/// plain click and the caller keeps it — this crate holds no anchor state.
/// The caller hands this function the anchor's *row*, and the row is clamped
/// into the list here: a stale anchor, one that names a row the list no
/// longer has because a removal shortened the list or a reorder moved the
/// row, can never select a row outside the list, and an empty list refuses
/// the gesture altogether. The caller therefore re-derives the anchor's row
/// with [`Playlist::index_of`] after a reorder *or a removal*; an anchor
/// whose entry is gone has no row, and the caller drops it.
pub fn select_range(playlist: &mut Playlist, anchor: usize, target: usize) -> bool {
    let len = playlist.len();
    if len == 0 {
        return false;
    }
    let (first, last) = ordered_clamped(anchor, target, len);
    let ids: Vec<EntryId> = playlist.entries()[first..=last]
        .iter()
        .map(|entry| entry.id)
        .collect();
    playlist.select_only(&ids);
    true
}

/// The ids a shift-click from `anchor` to `target` selects, over a bare entry
/// list.
///
/// The playlist-window machine holds the entry order as [`EntryId`]s rather
/// than a core [`Playlist`]; this is the same clamped range as [`select_range`],
/// so the two can never disagree. An empty list yields no ids.
pub fn range_ids(entries: &[EntryId], anchor: usize, target: usize) -> Vec<EntryId> {
    if entries.is_empty() {
        return Vec::new();
    }
    let (first, last) = ordered_clamped(anchor, target, entries.len());
    entries[first..=last].to_vec()
}

/// Ctrl-click: toggle the entry at `index` in or out of the selection.
///
/// Returns `false` when `index` is past the end. **Own choice** (the spec does
/// not settle multi-selection; see the module docs): the ordinary desktop
/// ctrl-click meaning, on top of the core [`Playlist::toggle_selected`].
pub fn toggle_select_at(playlist: &mut Playlist, index: usize) -> bool {
    let Some(id) = playlist.get_index(index).map(|entry| entry.id) else {
        return false;
    };
    playlist.toggle_selected(id);
    true
}

/// Drag the selected block so its first entry ends up at `to`.
///
/// `to` is the index in the list *without* the moved entries — the convention
/// of [`Playlist::move_entries`] and of `Command::MoveEntries`. The block keeps
/// its relative order; the selection and the current entry are untouched, so
/// the current entry stays the same entry and only its index moves with its
/// row. Returns `false` when there is no selection or the order does not
/// change.
///
/// **Own choice** (the drag is provisional in S03 §5 and undecided in S06 §8
/// Q6; see the module docs): the whole selection moves as one block.
pub fn move_selected(playlist: &mut Playlist, to: usize) -> bool {
    let selected = playlist.selected_in_order();
    if selected.is_empty() {
        return false;
    }
    let before: Vec<EntryId> = playlist.iter().map(|entry| entry.id).collect();
    playlist.move_entries(&selected, to);
    let after: Vec<EntryId> = playlist.iter().map(|entry| entry.id).collect();
    before != after
}

/// `Alt+Up` (`40215`, provisional): move the selected block one row up.
///
/// Returns `false` when nothing is selected or when the move does not change
/// the order — for a contiguous block that is the case when the block
/// already starts the list. **Own choice** (a provisional row; see the module
/// docs): the whole block moves, and a non-contiguous selection moves as one
/// block, so the gaps between the selected rows close.
pub fn move_selected_up(playlist: &mut Playlist) -> bool {
    let Some((first, _, _)) = selection_bounds(playlist) else {
        return false;
    };
    move_selected(playlist, first.saturating_sub(1))
}

/// `Alt+Down` (`40216`, provisional): move the selected block one row down.
///
/// Returns `false` when nothing is selected or when the move does not change
/// the order — for a contiguous block that is the case when the block
/// already ends the list. **Own choice**, as [`move_selected_up`].
pub fn move_selected_down(playlist: &mut Playlist) -> bool {
    let Some((_, last, count)) = selection_bounds(playlist) else {
        return false;
    };
    // After the selected entries are taken out, the entry that followed the
    // block sits `count` rows higher, so the block's new insertion index is
    // `last + 2 - count`. It never underflows: `count <= last + 1`.
    move_selected(playlist, last + 2 - count)
}

/// `Delete` (`1034`, "Remove the selected item(s)"): remove every selected
/// entry, returning the removed ids in playlist order.
///
/// An empty selection is a no-op returning no ids (S06 §5.3: without a staged
/// selection the removal is a no-op). The current entry follows the core
/// [`Playlist::remove`] contract: if it was removed, `current` becomes `None`;
/// every surviving entry keeps its id, so its index shifts down by however many
/// removed entries preceded it.
///
/// The spec settles *that* the selection is deleted, not this index rule (see
/// the module docs), so the rule — the core contract, unchanged — is stated
/// here as the own choice it is.
pub fn remove_selected(playlist: &mut Playlist) -> Vec<EntryId> {
    let selected = playlist.selected_in_order();
    let mut removed = Vec::with_capacity(selected.len());
    for id in selected {
        if playlist.remove(id).is_some() {
            removed.push(id);
        }
    }
    removed
}

/// `Ctrl+Delete` (`1035`, "Crop to the selected item(s)"): remove every entry
/// that is *not* selected, returning the removed ids in playlist order.
///
/// A no-op with no selection, matching the unstaged `Delete` of S06 §5.3; the
/// spec says nothing about an unstaged crop, so that is an **own choice**. The
/// current entry follows the same core rule as [`remove_selected`].
pub fn crop_to_selected(playlist: &mut Playlist) -> Vec<EntryId> {
    if playlist.selection().is_empty() {
        return Vec::new();
    }
    let doomed: Vec<EntryId> = playlist
        .iter()
        .map(|entry| entry.id)
        .filter(|id| !playlist.is_selected(*id))
        .collect();
    let mut removed = Vec::with_capacity(doomed.len());
    for id in doomed {
        if playlist.remove(id).is_some() {
            removed.push(id);
        }
    }
    removed
}

/// The inclusive, ordered index range `anchor..=target`, both ends clamped to
/// the last row of a list of `len` entries.
///
/// Callers guarantee `len > 0`.
fn ordered_clamped(anchor: usize, target: usize, len: usize) -> (usize, usize) {
    let a = anchor.min(len - 1);
    let b = target.min(len - 1);
    if a <= b { (a, b) } else { (b, a) }
}

/// `(first_index, last_index, selected_count)` of the selection, in playlist
/// order; `None` when nothing is selected.
fn selection_bounds(playlist: &Playlist) -> Option<(usize, usize, usize)> {
    let selected = playlist.selected_in_order();
    let first = playlist.index_of(*selected.first()?)?;
    let last = playlist.index_of(*selected.last()?)?;
    Some((first, last, selected.len()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustamp_core::playlist::{EntryId, MediaLocation, Playlist};

    use super::*;

    fn playlist_with(names: &[&str]) -> Playlist {
        let mut playlist = Playlist::new();
        for name in names {
            playlist.push(MediaLocation::File(PathBuf::from(*name)));
        }
        playlist
    }

    fn entry_ids(playlist: &Playlist) -> Vec<EntryId> {
        playlist.iter().map(|entry| entry.id).collect()
    }

    fn names(playlist: &Playlist) -> Vec<String> {
        playlist
            .iter()
            .map(|entry| entry.location.file_name())
            .collect()
    }

    #[test]
    fn a_plain_click_selects_one_row_and_replaces_the_selection() {
        // S06 §2 (settled): a single click selects one row; restated for the
        // model as a single-entry replacement of the selection set.
        let mut p = playlist_with(&["a", "b", "c"]);
        let ids = entry_ids(&p);
        select_range(&mut p, 0, 2); // all three selected first
        assert!(click_select(&mut p, 1));
        assert_eq!(p.selected_in_order(), vec![ids[1]]);

        // A row past the end clicks nothing, and an empty playlist refuses every
        // index.
        assert!(!click_select(&mut p, 3));
        assert_eq!(p.selected_in_order(), vec![ids[1]]);
        let mut empty = Playlist::new();
        assert!(!click_select(&mut empty, 0));

        // The click does not touch the current entry.
        p.set_current(Some(ids[0]));
        click_select(&mut p, 2);
        assert_eq!(p.current(), Some(ids[0]));
    }

    #[test]
    fn a_shift_click_selects_the_contiguous_range() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e", "f"]);
        let ids = entry_ids(&p);

        // Forward range.
        assert!(select_range(&mut p, 1, 4));
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2], ids[3], ids[4]]);

        // Reversed arguments give the same range.
        select_range(&mut p, 4, 1);
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2], ids[3], ids[4]]);

        // Shift-click replaces the previous selection; it does not extend it.
        select_range(&mut p, 0, 0);
        assert_eq!(p.selected_in_order(), vec![ids[0]]);

        // Both ends clamp to the last row: a click below the list reaches the
        // end of it.
        select_range(&mut p, 3, 99);
        assert_eq!(p.selected_in_order(), vec![ids[3], ids[4], ids[5]]);
        select_range(&mut p, 99, 99);
        assert_eq!(p.selected_in_order(), vec![ids[5]]);
    }

    #[test]
    fn a_ctrl_click_toggles_one_entry_and_keeps_the_rest() {
        let mut p = playlist_with(&["a", "b", "c", "d"]);
        let ids = entry_ids(&p);
        select_range(&mut p, 0, 1); // a b selected
        assert!(toggle_select_at(&mut p, 2)); // + c
        assert_eq!(p.selected_in_order(), vec![ids[0], ids[1], ids[2]]);
        assert!(toggle_select_at(&mut p, 0)); // - a
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2]]);

        // A row past the end toggles nothing.
        assert!(!toggle_select_at(&mut p, 9));
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2]]);
    }

    #[test]
    fn a_drag_moves_the_playing_entry_with_its_row() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e"]);
        let ids = entry_ids(&p);
        p.set_current(Some(ids[1])); // b plays
        p.select_only(&[ids[1], ids[2]]); // b c selected

        // Drag the block down so it lands after d: without b and c the list is
        // [a, d, e], so `to = 2` yields [a, d, b, c, e].
        assert!(move_selected(&mut p, 2));
        assert_eq!(names(&p), ["a", "d", "b", "c", "e"]);

        // The current *entry* is unchanged - no lookalike took its place ...
        assert_eq!(p.current(), Some(ids[1]));
        // ... and its index followed its row (b is row 2 now).
        assert_eq!(p.current_index(), Some(2));
        // The block stayed selected and contiguous.
        assert_eq!(p.selected_in_order(), vec![ids[1], ids[2]]);

        // A reorder is a permutation: every id is still there exactly once.
        let mut after = entry_ids(&p);
        after.sort();
        let mut before = entry_ids(&p);
        before.sort();
        assert_eq!(after, before);

        // Moving the block to where it already is changes nothing and says so.
        assert!(!move_selected(&mut p, 2));
        // Moving it to the very top.
        assert!(move_selected(&mut p, 0));
        assert_eq!(names(&p), ["b", "c", "a", "d", "e"]);
        assert_eq!(p.current_index(), Some(0));
    }

    #[test]
    fn alt_up_and_alt_down_move_the_selected_block_one_row() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e"]);
        let ids = entry_ids(&p);
        p.set_current(Some(ids[2])); // c plays

        p.select_only(&[ids[2]]); // c
        assert!(move_selected_up(&mut p));
        assert_eq!(names(&p), ["a", "c", "b", "d", "e"]);
        // The playing entry is still the same entry, one row up.
        assert_eq!(p.current(), Some(ids[2]));
        assert_eq!(p.current_index(), Some(1));
        assert!(move_selected_down(&mut p));
        assert_eq!(names(&p), ["a", "b", "c", "d", "e"]);
        assert_eq!(p.current_index(), Some(2));

        // A contiguous block moves as a block.
        p.select_only(&[ids[0], ids[1]]); // a b
        assert!(move_selected_down(&mut p));
        assert_eq!(names(&p), ["c", "a", "b", "d", "e"]);
        assert!(move_selected_up(&mut p));
        assert_eq!(names(&p), ["a", "b", "c", "d", "e"]);

        // At the ends the move is refused and the order is untouched.
        p.select_only(&[ids[0]]);
        assert!(!move_selected_up(&mut p));
        p.select_only(&[ids[4]]);
        assert!(!move_selected_down(&mut p));
        assert_eq!(names(&p), ["a", "b", "c", "d", "e"]);

        // With nothing selected there is nothing to move.
        p.select_none();
        assert!(!move_selected_up(&mut p));
        assert!(!move_selected_down(&mut p));
    }

    #[test]
    fn deleting_the_selection_shifts_the_current_index_by_what_preceded_it() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e", "f"]);
        let ids = entry_ids(&p);
        p.set_current(Some(ids[4])); // e plays
        p.select_only(&[ids[1], ids[2]]); // b c selected

        // Delete removes them in playlist order and returns their ids.
        assert_eq!(remove_selected(&mut p), vec![ids[1], ids[2]]);
        assert_eq!(names(&p), ["a", "d", "e", "f"]);
        // Two entries before e are gone, so e moved from index 4 to 2 - the
        // same entry, not a replacement.
        assert_eq!(p.current(), Some(ids[4]));
        assert_eq!(p.current_index(), Some(2));
        // The deleted entries left the selection behind.
        assert!(p.selected_in_order().is_empty());

        // Deleting the current entry clears it (the core `remove` contract).
        p.select_only(&[ids[4]]);
        assert_eq!(remove_selected(&mut p), vec![ids[4]]);
        assert_eq!(p.current(), None);
        assert_eq!(names(&p), ["a", "d", "f"]);

        // Delete with nothing staged is a no-op (S06 §5.3).
        assert!(remove_selected(&mut p).is_empty());
        assert_eq!(names(&p), ["a", "d", "f"]);
    }

    #[test]
    fn crop_keeps_only_the_selection_and_removes_the_rest_in_order() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e"]);
        let ids = entry_ids(&p);
        p.set_current(Some(ids[2])); // c survives
        p.select_only(&[ids[0], ids[2]]);

        assert_eq!(crop_to_selected(&mut p), vec![ids[1], ids[3], ids[4]]);
        assert_eq!(names(&p), ["a", "c"]);
        assert_eq!(p.current(), Some(ids[2]));
        assert_eq!(p.current_index(), Some(1));
        assert_eq!(p.selected_in_order(), vec![ids[0], ids[2]]);

        // An unstaged crop is a no-op (own choice, mirroring S06 §5.3).
        p.select_none();
        assert!(crop_to_selected(&mut p).is_empty());
        assert_eq!(names(&p), ["a", "c"]);

        // A crop that drops the current entry clears it.
        let mut q = playlist_with(&["a", "b", "c"]);
        let qids = entry_ids(&q);
        q.set_current(Some(qids[1]));
        q.select_only(&[qids[0]]);
        assert_eq!(crop_to_selected(&mut q), vec![qids[1], qids[2]]);
        assert_eq!(q.current(), None);
    }

    #[test]
    fn empty_and_single_entry_playlists_take_every_interaction() {
        let mut empty = Playlist::new();
        assert!(!click_select(&mut empty, 0));
        assert!(!select_range(&mut empty, 0, 3));
        assert!(range_ids(&[], 0, 3).is_empty());
        assert!(!toggle_select_at(&mut empty, 0));
        assert!(!move_selected(&mut empty, 0));
        assert!(!move_selected_up(&mut empty));
        assert!(!move_selected_down(&mut empty));
        assert!(remove_selected(&mut empty).is_empty());
        assert!(crop_to_selected(&mut empty).is_empty());

        let mut one = playlist_with(&["only"]);
        let id = entry_ids(&one)[0];
        one.set_current(Some(id));
        // The single row answers a click.
        assert!(click_select(&mut one, 0));
        assert_eq!(one.selected_in_order(), vec![id]);
        // Any range clamps onto the single row.
        assert!(select_range(&mut one, 0, 9));
        assert_eq!(one.selected_in_order(), vec![id]);
        // One row cannot be reordered.
        assert!(!move_selected_up(&mut one));
        assert!(!move_selected_down(&mut one));
        assert!(!move_selected(&mut one, 0));
        // It can be deleted, which clears the current entry.
        assert_eq!(remove_selected(&mut one), vec![id]);
        assert!(one.is_empty());
        assert_eq!(one.current(), None);
    }

    #[test]
    fn range_ids_matches_select_range_over_a_bare_id_list() {
        let mut p = playlist_with(&["a", "b", "c", "d", "e"]);
        let ids = entry_ids(&p);

        assert_eq!(range_ids(&ids, 1, 3), vec![ids[1], ids[2], ids[3]]);
        assert_eq!(range_ids(&ids, 3, 1), vec![ids[1], ids[2], ids[3]]);
        assert_eq!(range_ids(&ids, 3, 99), vec![ids[3], ids[4]]);
        assert_eq!(range_ids(&ids, 99, 99), vec![ids[4]]);
        assert!(range_ids(&[], 0, 2).is_empty());

        // Both entry points agree on the same range.
        select_range(&mut p, 1, 3);
        assert_eq!(p.selected_in_order(), range_ids(&ids, 1, 3));
    }
}
