// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The skin's cursors: which one a window shows, and the cache of the
//! operating system's cursors.
//!
//! The mapping from a window region to a cursor slot is `rustamp-ui`'s (see
//! `rustamp_ui::cursor`); this module is the half that owns the skin: it
//! looks the slot up in the current skin, turns the skin's file into an
//! operating-system cursor and keeps that cursor for as long as the skin
//! lives.
//!
//! # The rules
//!
//! * A slot the skin carries becomes the window's cursor.
//! * A slot the skin does not carry leaves the system's own cursor in
//!   place: no `SetCursor` call is made at all, so the pointer keeps
//!   whatever the system shows.
//! * A file the operating system refuses is treated like a missing file:
//!   the system's own cursor stays. S17 §8 leaves the question open ("a
//!   cursor file of the wrong size"); refusing to show a broken file and
//!   falling back to the system cursor is the answer this player gives.
//!
//! # The headless seam
//!
//! [`cursor_action`] is pure: it turns "the skin's file for this slot" and
//! "could the operating system use it" into the action to take, and it is
//! unit-tested without an operating system and without a window. Showing a
//! cursor needs a desktop, so the `SetCursor` half is VM-only.

use std::collections::HashMap;

use rustamp_platform::cursor::{CursorFormat, OsCursor};
use rustamp_skin::{CursorFile, CursorKind, CursorSlot};

/// What the app does with the cursor at a pointer position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorAction {
    /// Show the skin's own cursor for this slot.
    Show(CursorSlot),
    /// Leave the operating system's cursor in place.
    KeepSystem,
}

/// The action for `slot`.
///
/// `file` is the skin's cursor file for the slot - `None` when the skin
/// carries none. `usable` says whether the operating system could turn
/// that file into a cursor. A file it refused leaves the system's cursor
/// in place, exactly like a missing file.
pub const fn cursor_action(
    slot: CursorSlot,
    file: Option<&CursorFile>,
    usable: bool,
) -> CursorAction {
    match (file, usable) {
        (Some(_), true) => CursorAction::Show(slot),
        _ => CursorAction::KeepSystem,
    }
}

/// What the cache holds for one slot.
enum SlotCursor {
    /// The operating system built a cursor from the skin's file.
    Ready(OsCursor),
    /// The skin carries a file, but the operating system refused it.
    Refused,
}

/// The operating-system cursors of the current skin, one per slot.
///
/// The cache belongs to one skin load: [`SkinCursors::sync`] drops every
/// cursor it holds when the app loaded another skin, so no cursor handle
/// outlives the skin it was built from.
pub struct SkinCursors {
    /// The skin load the cached cursors belong to.
    generation: u64,
    /// The cursor per slot, built on the first ask.
    cursors: HashMap<CursorSlot, SlotCursor>,
}

impl SkinCursors {
    /// A cache for the skin load `generation`.
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            cursors: HashMap::new(),
        }
    }

    /// Drops the cached cursors when the app loaded another skin.
    ///
    /// The comparison is the whole cost, so this can run on every pointer
    /// move.
    pub fn sync(&mut self, generation: u64) {
        if self.generation != generation {
            self.cursors.clear();
            self.generation = generation;
        }
    }

    /// The operating-system cursor for `slot`.
    ///
    /// `file` is the skin's cursor file for the slot. The cursor is built
    /// on the first ask and kept until the skin changes. `None` when the
    /// skin carries no file for the slot, or when the operating system
    /// refused it.
    pub fn get(&mut self, slot: CursorSlot, file: Option<&CursorFile>) -> Option<&OsCursor> {
        // A slot the skin does not carry is not cached: there is nothing
        // to build, and the answer cannot change while the skin lives.
        let file = file?;
        self.cursors.entry(slot).or_insert_with(|| {
            let built = OsCursor::build(&file.bytes, container(file.kind)).ok();
            built.map(SlotCursor::Ready).unwrap_or(SlotCursor::Refused)
        });
        match self.cursors.get(&slot)? {
            SlotCursor::Ready(cursor) => Some(cursor),
            SlotCursor::Refused => None,
        }
    }

    /// The number of slots the cache holds, for the tests.
    ///
    /// A slot is held even when its file was refused: the refusal is
    /// cached too, so the operating system is not asked again on every
    /// pointer move.
    #[cfg(test)]
    pub fn cached_slots(&self) -> usize {
        self.cursors.len()
    }
}

/// The container a skin's cursor file uses.
fn container(kind: CursorKind) -> CursorFormat {
    match kind {
        CursorKind::Cur => CursorFormat::Cur,
        CursorKind::Ani => CursorFormat::Ani,
    }
}

#[cfg(test)]
mod tests {
    use rustamp_skin::{BuiltinLookalikeSkin, MemorySource, Skin};

    use super::*;

    /// A cursor file the operating system cannot use: too short to be a
    /// cursor at all.
    fn broken_file(name: &str) -> CursorFile {
        CursorFile {
            name: name.to_string(),
            bytes: b"nope".to_vec(),
            kind: CursorKind::Cur,
        }
    }

    /// A cursor file with a well-formed directory: the operating system is
    /// asked about it, and the answer is what the test says it is.
    fn good_file(name: &str) -> CursorFile {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes()); // idReserved
        bytes.extend_from_slice(&2u16.to_le_bytes()); // idType: a cursor
        bytes.extend_from_slice(&1u16.to_le_bytes()); // idCount
        bytes.extend_from_slice(&[32, 32, 0, 0]); // size, colours, reserved
        bytes.extend_from_slice(&8u16.to_le_bytes()); // hotspot x
        bytes.extend_from_slice(&8u16.to_le_bytes()); // hotspot y
        bytes.extend_from_slice(&0u32.to_le_bytes()); // bytes in the image
        bytes.extend_from_slice(&22u32.to_le_bytes()); // offset of the image
        CursorFile {
            name: name.to_string(),
            bytes,
            kind: CursorKind::Cur,
        }
    }

    /// A skin that carries exactly the given cursor files and nothing else.
    ///
    /// The look-alike fallback supplies no cursors, so the skin carries
    /// only what the test put in it.
    fn skin_with(files: &[CursorFile]) -> Skin {
        let mut source = MemorySource::new("cursor test skin");
        for file in files {
            source.add(file.name.clone(), file.bytes.clone());
        }
        Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the test skin loads")
    }

    #[test]
    fn a_slot_the_skin_carries_shows_the_skins_own_cursor() {
        let skin = skin_with(&[good_file("normal.cur")]);
        let file = skin.cursor_slot(CursorSlot::Normal);
        assert!(file.is_some(), "the skin carries normal.cur");
        assert_eq!(
            cursor_action(CursorSlot::Normal, file, true),
            CursorAction::Show(CursorSlot::Normal)
        );
    }

    #[test]
    fn a_slot_the_skin_does_not_carry_keeps_the_system_cursor() {
        let skin = skin_with(&[]);
        let file = skin.cursor_slot(CursorSlot::Normal);
        assert!(file.is_none(), "the skin carries no cursor at all");
        assert_eq!(
            cursor_action(CursorSlot::Normal, file, false),
            CursorAction::KeepSystem
        );
    }

    #[test]
    fn a_file_the_operating_system_refused_keeps_the_system_cursor() {
        let file = good_file("posbar.cur");
        assert_eq!(
            cursor_action(CursorSlot::PosBar, Some(&file), false),
            CursorAction::KeepSystem
        );
    }

    #[test]
    fn the_cache_holds_no_cursor_for_a_slot_the_skin_does_not_carry() {
        let skin = skin_with(&[]);
        let mut cursors = SkinCursors::new(0);
        let file = skin.cursor_slot(CursorSlot::Normal);
        assert!(cursors.get(CursorSlot::Normal, file).is_none());
        assert_eq!(cursors.cached_slots(), 0);
    }

    #[test]
    fn the_cache_refuses_a_broken_cursor_file_and_keeps_the_refusal() {
        // The file is too short to be a cursor, so the operating system is
        // never asked: the skin's own cursor is refused up front.
        let skin = skin_with(&[broken_file("normal.cur")]);
        let file = skin.cursor_slot(CursorSlot::Normal);
        let mut cursors = SkinCursors::new(0);
        assert!(cursors.get(CursorSlot::Normal, file).is_none());
        // The refusal is kept, so the next pointer move does not ask the
        // operating system about the same broken file again.
        assert_eq!(cursors.cached_slots(), 1);
        assert!(cursors.get(CursorSlot::Normal, file).is_none());
        assert_eq!(cursors.cached_slots(), 1);
    }

    #[test]
    fn the_built_in_skins_cursors_become_operating_system_cursors() {
        // The operating system's own loader takes the built-in skin's
        // `normal.cur`; a file it refuses would leave every window with
        // the system cursor and no skin cursor at all.
        let skin = crate::skin::builtin();
        let file = skin
            .skin()
            .cursor_slot(CursorSlot::Normal)
            .expect("the built-in skin carries normal.cur");
        let cursor = OsCursor::build(&file.bytes, container(file.kind));
        assert!(cursor.is_ok(), "{:?}", cursor.err());
    }

    #[test]
    fn another_skin_drops_the_cached_cursors() {
        let skin = skin_with(&[broken_file("normal.cur")]);
        let file = skin.cursor_slot(CursorSlot::Normal);
        let mut cursors = SkinCursors::new(1);
        cursors.get(CursorSlot::Normal, file);
        assert_eq!(cursors.cached_slots(), 1);
        // The same skin keeps the cache; another one empties it, so no
        // cursor of the old skin outlives it.
        cursors.sync(1);
        assert_eq!(cursors.cached_slots(), 1);
        cursors.sync(2);
        assert_eq!(cursors.cached_slots(), 0);
    }
}
