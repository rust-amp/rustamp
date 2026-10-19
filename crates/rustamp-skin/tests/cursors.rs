// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The cursor-slot model: which slot a skin's cursor files fill, and what a
//! skin without cursors reports.
//!
//! S17 §8 verified that the base skin ships eleven cursor files and that
//! they are ordinary Windows `.cur` files. The model here names the slots and
//! hands out the file's bytes; it never decodes them.

use rustamp_skin::{BuiltinLookalikeSkin, CursorKind, CursorSlot, MemorySource, Skin};

/// The cursor bytes the synthetic skin carries, one per slot name.
fn cursor_bytes(stem: &str) -> Vec<u8> {
    format!("cursor-file:{stem}").into_bytes()
}

/// Builds a skin that carries the given `(file name, bytes)` cursor entries
/// and nothing else, with a fallback that ships no cursors at all.
fn skin_with_cursors(entries: &[(String, Vec<u8>)]) -> Skin {
    let mut source = MemorySource::new("cursor skin");
    for (name, bytes) in entries {
        source.add(name.clone(), bytes.clone());
    }
    Skin::from_source(&mut source, &BuiltinLookalikeSkin).expect("the skin loads")
}

/// A skin that ships a cursor file exposes the slot behind it, with the
/// file's bytes untouched.
#[test]
fn a_skin_with_cursors_exposes_every_slot_it_ships() {
    let entries: Vec<(String, Vec<u8>)> = [
        ("normal.cur", "normal.cur"),
        ("close.cur", "close.cur"),
        ("posbar.cur", "posbar.cur"),
        ("volbal.cur", "volbal.cur"),
        ("titlebar.cur", "titlebar.cur"),
        ("eqslid.cur", "eqslid.cur"),
        ("eqtitle.cur", "eqtitle.cur"),
        ("pnormal.cur", "pnormal.cur"),
        ("ptbar.cur", "ptbar.cur"),
        ("pvscroll.cur", "pvscroll.cur"),
        ("psize.cur", "psize.cur"),
    ]
    .iter()
    .map(|(name, stem)| (name.to_string(), cursor_bytes(stem)))
    .collect();
    let skin = skin_with_cursors(&entries);

    for (name, bytes) in &entries {
        let slot = CursorSlot::from_file_name(name).expect("a known slot");
        let cursor = skin
            .cursor_slot(slot)
            .unwrap_or_else(|| panic!("slot {slot:?} must be filled"));
        assert_eq!(cursor.bytes, *bytes, "slot {slot:?}");
        assert_eq!(cursor.kind, CursorKind::Cur, "slot {slot:?}");
    }
}

/// The eleven cursors of S17 §8 fill exactly the slots of the same name.
#[test]
fn the_base_skins_eleven_cursors_fill_their_slots() {
    let entries: Vec<(String, Vec<u8>)> = [
        "normal", "close", "eqslid", "eqtitle", "posbar", "psize", "ptbar", "pvscroll", "titlebar",
        "volbal", "pnormal",
    ]
    .iter()
    .map(|stem| (format!("{stem}.cur"), cursor_bytes(stem)))
    .collect();
    let skin = skin_with_cursors(&entries);

    for (name, bytes) in &entries {
        let slot = CursorSlot::from_file_name(name).expect("a known slot");
        assert_eq!(
            skin.cursor_slot(slot).map(|c| c.bytes.as_slice()),
            Some(bytes.as_slice()),
            "slot {slot:?}"
        );
    }
    // Every other slot stays absent.
    for slot in CursorSlot::ALL {
        if entries
            .iter()
            .any(|(name, _)| CursorSlot::from_file_name(name) == Some(*slot))
        {
            continue;
        }
        assert!(
            skin.cursor_slot(*slot).is_none(),
            "slot {slot:?} must be absent"
        );
    }
}

/// A skin that ships no cursor at all reports every slot as absent, so the
/// caller falls back to the system cursor.
#[test]
fn a_skin_without_cursors_reports_every_slot_absent() {
    let skin = skin_with_cursors(&[]);
    assert!(skin.cursors().is_empty());
    for slot in CursorSlot::ALL {
        assert!(
            skin.cursor_slot(*slot).is_none(),
            "slot {slot:?} must be absent"
        );
    }
}

/// The slot names are matched case-insensitively, like every other skin file.
#[test]
fn cursor_file_names_are_matched_case_insensitively() {
    let skin = skin_with_cursors(&[("CLOSE.CUR".to_string(), cursor_bytes("close"))]);
    let cursor = skin
        .cursor_slot(CursorSlot::Close)
        .expect("the upper-case file fills the slot");
    assert_eq!(cursor.bytes, cursor_bytes("close"));
}

/// A skin that ships `posbar.ani` fills the `posbar` slot: the `.ani`
/// variant is the same slot, and the bytes are handed out as they are.
#[test]
fn an_ani_file_fills_the_slot_of_its_cur_name() {
    let skin = skin_with_cursors(&[("posbar.ani".to_string(), b"RIFF-animated".to_vec())]);
    let cursor = skin
        .cursor_slot(CursorSlot::PosBar)
        .expect("the .ani file fills the posbar slot");
    assert_eq!(cursor.bytes, b"RIFF-animated");
    assert_eq!(cursor.kind, CursorKind::Ani);
}

/// Every slot of the model can be filled by a skin that ships the whole
/// classic set.
#[test]
fn a_skin_can_fill_every_slot_of_the_model() {
    let entries: Vec<(String, Vec<u8>)> = CursorSlot::ALL
        .iter()
        .map(|slot| (slot.file_name().to_string(), cursor_bytes(slot.stem())))
        .collect();
    let skin = skin_with_cursors(&entries);

    for slot in CursorSlot::ALL {
        let cursor = skin
            .cursor_slot(*slot)
            .unwrap_or_else(|| panic!("slot {slot:?} must be filled"));
        assert_eq!(cursor.bytes, cursor_bytes(slot.stem()), "slot {slot:?}");
    }
    assert_eq!(skin.cursors().len(), CursorSlot::ALL.len());
}
