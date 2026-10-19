// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The classic cursor slots: the cursor files a skin may ship.
//!
//! A **slot** is the name a cursor is carried under, not a decoded image. The
//! file's bytes stay untouched: the OS loads them, so this crate never
//! decodes a `.cur` or an `.ani` and needs no decoder for them (S17 §8
//! verified that the base skin's cursors are ordinary Windows `.cur` files,
//! but says nothing about their content).
//!
//! S17 §8 verified the eleven cursors the base skin ships and left two
//! things open: which window region each cursor belongs to, and what happens
//! when a cursor is missing. The **names** below are the classic set the
//! loader asks for (ported from webamp's `CURSORS` table, MIT — see
//! `THIRD_PARTY.md`); the mapping from a window region to a slot is **not**
//! settled here and lives in `rustamp-ui` as Rustamp's own choice.
//!
//! A slot is filled by the file that carries it — `<stem>.cur` or
//! `<stem>.ani`, matched case-insensitively — and a slot nobody carries stays
//! absent, so the caller can fall back to the system cursor.

/// One classic cursor slot.
///
/// The slot names the *file* a cursor lives in; the file's bytes are
/// [`crate::CursorFile::bytes`] and are never decoded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorSlot {
    /// `close.cur` — the main window's close button.
    Close,
    /// `eqclose.cur` — the equalizer window's close button.
    EqClose,
    /// `eqnormal.cur` — the equalizer window's background.
    EqNormal,
    /// `eqslid.cur` — the equalizer window's sliders.
    EqSlid,
    /// `eqtitle.cur` — the equalizer window's title bar.
    EqTitle,
    /// `mainmenu.cur` — the main window's menu button.
    MainMenu,
    /// `mmenu.cur` — an item of the open main menu.
    MMenu,
    /// `min.cur` — the main window's minimize button.
    Min,
    /// `normal.cur` — the main window's background.
    Normal,
    /// `pclose.cur` — the playlist window's close button.
    PClose,
    /// `pnormal.cur` — the playlist window's background.
    PNormal,
    /// `posbar.cur` — the main window's position slider.
    PosBar,
    /// `psize.cur` — the playlist window's resize grip.
    PSize,
    /// `ptbar.cur` — the playlist window's title bar.
    PTBar,
    /// `pvscroll.cur` — the playlist window's vertical scrollbar.
    PVScroll,
    /// `pwinbut.cur` — the playlist window's window button.
    PWinBut,
    /// `pwsnorm.cur` — the playlist window in its windowshade form.
    PWSNorm,
    /// `pwssize.cur` — the playlist windowshade's size grip.
    PWSSize,
    /// `songname.cur` — the main window's song-title text.
    SongName,
    /// `titlebar.cur` — the main window's title bar.
    TitleBar,
    /// `volbal.cur` — the main window's volume and balance sliders.
    VolBal,
    /// `winbut.cur` — the main window's window button.
    WinBut,
    /// `wsnormal.cur` — the main window in its windowshade form.
    WSNormal,
    /// `wsposbar.cur` — the main windowshade's position slider.
    WSPosBar,
}

impl CursorSlot {
    /// Every slot, in the order the loader asks for them.
    pub const ALL: &'static [CursorSlot] = &[
        CursorSlot::Close,
        CursorSlot::EqClose,
        CursorSlot::EqNormal,
        CursorSlot::EqSlid,
        CursorSlot::EqTitle,
        CursorSlot::MainMenu,
        CursorSlot::MMenu,
        CursorSlot::Min,
        CursorSlot::Normal,
        CursorSlot::PClose,
        CursorSlot::PNormal,
        CursorSlot::PosBar,
        CursorSlot::PSize,
        CursorSlot::PTBar,
        CursorSlot::PVScroll,
        CursorSlot::PWinBut,
        CursorSlot::PWSNorm,
        CursorSlot::PWSSize,
        CursorSlot::SongName,
        CursorSlot::TitleBar,
        CursorSlot::VolBal,
        CursorSlot::WinBut,
        CursorSlot::WSNormal,
        CursorSlot::WSPosBar,
    ];

    /// The file name this slot is carried under, in its canonical spelling.
    pub const fn file_name(self) -> &'static str {
        match self {
            CursorSlot::Close => "close.cur",
            CursorSlot::EqClose => "eqclose.cur",
            CursorSlot::EqNormal => "eqnormal.cur",
            CursorSlot::EqSlid => "eqslid.cur",
            CursorSlot::EqTitle => "eqtitle.cur",
            CursorSlot::MainMenu => "mainmenu.cur",
            CursorSlot::MMenu => "mmenu.cur",
            CursorSlot::Min => "min.cur",
            CursorSlot::Normal => "normal.cur",
            CursorSlot::PClose => "pclose.cur",
            CursorSlot::PNormal => "pnormal.cur",
            CursorSlot::PosBar => "posbar.cur",
            CursorSlot::PSize => "psize.cur",
            CursorSlot::PTBar => "ptbar.cur",
            CursorSlot::PVScroll => "pvscroll.cur",
            CursorSlot::PWinBut => "pwinbut.cur",
            CursorSlot::PWSNorm => "pwsnorm.cur",
            CursorSlot::PWSSize => "pwssize.cur",
            CursorSlot::SongName => "songname.cur",
            CursorSlot::TitleBar => "titlebar.cur",
            CursorSlot::VolBal => "volbal.cur",
            CursorSlot::WinBut => "winbut.cur",
            CursorSlot::WSNormal => "wsnormal.cur",
            CursorSlot::WSPosBar => "wsposbar.cur",
        }
    }

    /// The file-name stem of this slot, without the `.cur`/`.ani` extension.
    ///
    /// This is the name the loader looks the slot up under: a skin may carry
    /// the slot as `<stem>.cur` or as `<stem>.ani`.
    pub const fn stem(self) -> &'static str {
        match self {
            CursorSlot::Close => "close",
            CursorSlot::EqClose => "eqclose",
            CursorSlot::EqNormal => "eqnormal",
            CursorSlot::EqSlid => "eqslid",
            CursorSlot::EqTitle => "eqtitle",
            CursorSlot::MainMenu => "mainmenu",
            CursorSlot::MMenu => "mmenu",
            CursorSlot::Min => "min",
            CursorSlot::Normal => "normal",
            CursorSlot::PClose => "pclose",
            CursorSlot::PNormal => "pnormal",
            CursorSlot::PosBar => "posbar",
            CursorSlot::PSize => "psize",
            CursorSlot::PTBar => "ptbar",
            CursorSlot::PVScroll => "pvscroll",
            CursorSlot::PWinBut => "pwinbut",
            CursorSlot::PWSNorm => "pwsnorm",
            CursorSlot::PWSSize => "pwssize",
            CursorSlot::SongName => "songname",
            CursorSlot::TitleBar => "titlebar",
            CursorSlot::VolBal => "volbal",
            CursorSlot::WinBut => "winbut",
            CursorSlot::WSNormal => "wsnormal",
            CursorSlot::WSPosBar => "wsposbar",
        }
    }

    /// The slot a cursor file name belongs to.
    ///
    /// The name is matched case-insensitively and the `.cur`/`.ani` extension
    /// is ignored, so `POSBAR.ani` fills the `posbar` slot. A name that
    /// belongs to no slot returns `None`.
    pub fn from_file_name(name: &str) -> Option<CursorSlot> {
        let lower = name.to_ascii_lowercase();
        let stem = match lower.strip_suffix(".cur") {
            Some(stem) => stem,
            None => lower.strip_suffix(".ani").unwrap_or(lower.as_str()),
        };
        CursorSlot::ALL
            .iter()
            .copied()
            .find(|slot| slot.stem() == stem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_has_a_unique_stem_and_file_name() {
        let mut stems: Vec<&str> = Vec::new();
        for slot in CursorSlot::ALL {
            assert_eq!(
                slot.file_name(),
                format!("{}.cur", slot.stem()),
                "{slot:?} must be carried under its own stem"
            );
            assert!(
                !stems.contains(&slot.stem()),
                "duplicate slot stem {}",
                slot.stem()
            );
            stems.push(slot.stem());
        }
    }

    #[test]
    fn a_file_name_resolves_to_its_slot() {
        assert_eq!(
            CursorSlot::from_file_name("posbar.cur"),
            Some(CursorSlot::PosBar)
        );
        // Case-insensitive, and the `.ani` variant fills the same slot.
        assert_eq!(
            CursorSlot::from_file_name("POSBAR.ANI"),
            Some(CursorSlot::PosBar)
        );
        assert_eq!(
            CursorSlot::from_file_name("Close.CUR"),
            Some(CursorSlot::Close)
        );
        // A name that is no classic cursor belongs to no slot.
        assert_eq!(CursorSlot::from_file_name("not-a-cursor.cur"), None);
        assert_eq!(CursorSlot::from_file_name("main.bmp"), None);
    }

    /// S17 §8: the base skin ships these eleven cursors; every one of them
    /// must be a slot the model knows.
    #[test]
    fn the_base_skin_cursor_set_is_covered() {
        for name in [
            "normal", "close", "eqslid", "eqtitle", "posbar", "psize", "ptbar", "pvscroll",
            "titlebar", "volbal", "pnormal",
        ] {
            let wanted = format!("{name}.cur");
            assert!(
                CursorSlot::ALL
                    .iter()
                    .any(|slot| slot.file_name() == wanted),
                "the model never asks for {wanted}"
            );
        }
    }
}
