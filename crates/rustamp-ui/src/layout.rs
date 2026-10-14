//! The main-window layout: every rectangle S01 measured, in one place.
//!
//! All rectangles are **window-relative** (x to the right, y down, origin at the
//! client top-left), exactly as in [`specs/S01-window-geometry.md`]. Each field
//! cites its S01 section. Values S01 marks **provisional** are called out in the
//! doc comment and in the field comment; they are not promoted to facts.
//!
//! The sprite rectangles in `rustamp-skin`'s table are the *source* rectangles
//! inside a sheet; the rectangles here are the *destinations* on the window.
//! Both come from S01 §2.1.

use rustamp_gfx::Rect;

use crate::state::WindowButton;

/// Width of the classic main window at 1x (S01 §1, verified).
pub const WINDOW_WIDTH: u32 = 275;
/// Height of the classic main window at 1x (S01 §1, verified).
pub const WINDOW_HEIGHT: u32 = 116;
/// Height of the windowshade form (S01 §1, verified).
pub const SHADE_HEIGHT: u32 = 14;
/// Width of the double-size form (S01 §1, verified: exactly 2x).
pub const DOUBLE_WIDTH: u32 = WINDOW_WIDTH * 2;
/// Height of the double-size form (S01 §1, verified: exactly 2x).
pub const DOUBLE_HEIGHT: u32 = WINDOW_HEIGHT * 2;

/// Every rectangle the classic main window draws, in one place.
///
/// The values in [`MainWindowLayout::CLASSIC`] are S01 §2.1/§2.2 destinations
/// plus clearly-marked additions S01 does not pin down: the marquee and
/// kbps/kHz field widths, the slider thumbs and the windowshade internals
/// (webamp-provisional), the time-display click band from S02 §4 and the four
/// window-button window positions (S02 §7 gives the pressed cells but not where
/// the buttons sit, so the positions stay webamp-provisional).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainWindowLayout {
    /// The opaque window background: `main.bmp` at (0,0) (S01 §2.1, verified).
    pub background: Rect,
    /// The title-bar band, focused and unfocused alike (S01 §2.1, verified).
    pub title_bar: Rect,
    /// The menu window button at (6,3), 9 x 9. **Provisional**: S02 §7 gives
    /// the sheet cell but not the window position; this position is the webamp
    /// cross-check.
    pub window_menu: Rect,
    /// The minimize window button at (244,3), 9 x 9 (S02 §7 cell; position
    /// provisional, webamp cross-check).
    pub window_minimize: Rect,
    /// The windowshade window button at (254,3), 9 x 9 (S02 §7 cell; position
    /// provisional, webamp cross-check).
    pub window_shade: Rect,
    /// The close window button at (264,3), 9 x 9 (S02 §7 cell; position
    /// provisional, webamp cross-check).
    pub window_close: Rect,
    /// The windowshade title-bar band, 275 x 14 (S01 §1/§2.1, provisional).
    pub shade_title_bar: Rect,
    /// The clutter-bar background, `titlebar.bmp` (304,0,8,43) at (10,22)
    /// (S01 §2.1, verified).
    pub clutter_bar: Rect,
    /// Clutter-bar letter `O` (S01 §2.1, provisional: never captured lit).
    pub clutter_o: Rect,
    /// Clutter-bar letter `A` (S01 §2.1, verified).
    pub clutter_a: Rect,
    /// Clutter-bar letter `I` (S01 §2.1, verified).
    pub clutter_i: Rect,
    /// Clutter-bar letter `D` (S01 §2.1, verified).
    pub clutter_d: Rect,
    /// Clutter-bar letter `V` (S01 §2.1, verified).
    pub clutter_v: Rect,
    /// Previous transport button (S01 §2.1, verified).
    pub previous: Rect,
    /// Play transport button (S01 §2.1, verified).
    pub play: Rect,
    /// Pause transport button (S01 §2.1, verified).
    pub pause: Rect,
    /// Stop transport button (S01 §2.1, verified).
    pub stop: Rect,
    /// Next transport button (S01 §2.1, verified).
    pub next: Rect,
    /// Eject transport button (S01 §2.1, verified).
    pub eject: Rect,
    /// Shuffle button (S01 §2.1, verified).
    pub shuffle: Rect,
    /// Repeat button (S01 §2.1, verified).
    pub repeat: Rect,
    /// Equalizer toggle button (S01 §2.1, verified).
    pub eq_toggle: Rect,
    /// Playlist toggle button (S01 §2.1, verified).
    pub playlist_toggle: Rect,
    /// Position-bar track, 248 x 10 (S01 §2.1, verified).
    pub position_track: Rect,
    /// Position-bar thumb **at position 0** (S01 §2.1/§2.4, provisional: the
    /// measured sheet x is 246 and the residual is unresolved).
    pub position_thumb: Rect,
    /// Volume track, 68 x 13 at (107,57) (S01 §2.1/open question 4,
    /// provisional).
    pub volume_track: Rect,
    /// Balance track, 38 x 13 at (177,57) (S01 §2.1/open question 4,
    /// provisional).
    pub balance_track: Rect,
    /// The four time-display digit cells at x = 48, 60, 78, 90, y = 26
    /// (S01 §2.1/§2.3, verified).
    pub time_digits: [Rect; 4],
    /// The remaining-time minus sign at (38,32), 5 x 1 (S01 §2.1, verified).
    pub minus_sign: Rect,
    /// The time display's click band at (36,26), 60 x 13 (S02 §4: the click area
    /// is the band around x = 36-96, y = 26-39).
    pub time_display: Rect,
    /// The play/pause/stop indicator at (26,28), 9 x 9 (S01 §2.1, verified).
    pub play_indicator: Rect,
    /// The decoder-active/idle indicator at (24,28), 3 x 9 (S01 §2.1, verified).
    pub decoder_indicator: Rect,
    /// The stereo indicator (S01 §2.1, verified).
    pub stereo: Rect,
    /// The mono indicator (S01 §2.1, verified).
    pub mono: Rect,
    /// The track-title marquee: first glyph at (111,27), 6 px tall (S01 §2.2,
    /// verified). The 154 px width is **provisional** (webamp); S01 gives the
    /// origin and the 5 px cell pitch but not the field's right edge.
    pub marquee: Rect,
    /// The kbps numeral field: first digit cell at (111,43) (S01 §2.2,
    /// verified). The 15 px width (three cells) is **provisional** (webamp).
    pub kbps: Rect,
    /// The kHz numeral field: first digit cell at (156,43) (S01 §2.2,
    /// verified). The 10 px width (two cells) is **provisional** (webamp).
    pub khz: Rect,
    /// The windowshade position slider at (226,4), 17 x 7. **Provisional**: S01
    /// does not cover the shade internals, so this comes from webamp.
    pub shade_position: Rect,
}

impl MainWindowLayout {
    /// The classic main-window layout (S01 §2).
    pub const CLASSIC: MainWindowLayout = MainWindowLayout {
        background: Rect::new(0, 0, 275, 116),
        title_bar: Rect::new(0, 0, 275, 14),
        window_menu: Rect::new(6, 3, 9, 9),
        window_minimize: Rect::new(244, 3, 9, 9),
        window_shade: Rect::new(254, 3, 9, 9),
        window_close: Rect::new(264, 3, 9, 9),
        shade_title_bar: Rect::new(0, 0, 275, 14),
        clutter_bar: Rect::new(10, 22, 8, 43),
        clutter_o: Rect::new(14, 25, 4, 6),
        clutter_a: Rect::new(14, 33, 4, 6),
        clutter_i: Rect::new(14, 41, 3, 6),
        clutter_d: Rect::new(14, 49, 4, 6),
        clutter_v: Rect::new(14, 56, 4, 6),
        previous: Rect::new(16, 88, 22, 18),
        play: Rect::new(39, 88, 22, 18),
        pause: Rect::new(62, 88, 22, 18),
        stop: Rect::new(85, 88, 22, 18),
        next: Rect::new(108, 88, 22, 18),
        eject: Rect::new(136, 89, 22, 16),
        shuffle: Rect::new(164, 89, 47, 15),
        repeat: Rect::new(210, 89, 28, 15),
        eq_toggle: Rect::new(219, 58, 23, 12),
        playlist_toggle: Rect::new(242, 58, 23, 12),
        position_track: Rect::new(16, 72, 248, 10),
        position_thumb: Rect::new(16, 72, 29, 10),
        volume_track: Rect::new(107, 57, 68, 13),
        balance_track: Rect::new(177, 57, 38, 13),
        time_digits: [
            Rect::new(48, 26, 9, 13),
            Rect::new(60, 26, 9, 13),
            Rect::new(78, 26, 9, 13),
            Rect::new(90, 26, 9, 13),
        ],
        minus_sign: Rect::new(38, 32, 5, 1),
        time_display: Rect::new(36, 26, 60, 13),
        play_indicator: Rect::new(26, 28, 9, 9),
        decoder_indicator: Rect::new(24, 28, 3, 9),
        stereo: Rect::new(239, 41, 29, 12),
        mono: Rect::new(212, 41, 27, 12),
        marquee: Rect::new(111, 27, 154, 6),
        kbps: Rect::new(111, 43, 15, 6),
        khz: Rect::new(156, 43, 10, 6),
        shade_position: Rect::new(226, 4, 17, 7),
    };

    /// The classic window bounds at 1x (S01 §1, verified).
    pub const fn window() -> Rect {
        Rect::new(0, 0, WINDOW_WIDTH, WINDOW_HEIGHT)
    }

    /// The windowshade bounds (S01 §1, verified height).
    pub const fn shade_window() -> Rect {
        Rect::new(0, 0, WINDOW_WIDTH, SHADE_HEIGHT)
    }

    /// The window position of a title-bar window button (S02 §7).
    ///
    /// The pressed-state **sheet cells** are verified in S02 §7; the window
    /// positions are not in S02, so they stay provisional (webamp cross-check).
    pub const fn window_button(&self, button: WindowButton) -> Rect {
        match button {
            WindowButton::Menu => self.window_menu,
            WindowButton::Minimize => self.window_minimize,
            WindowButton::Shade => self.window_shade,
            WindowButton::Close => self.window_close,
        }
    }

    /// The position-bar thumb rectangle for a 0..=100 position.
    ///
    /// At position 0 the thumb sits at the track's left edge, which S01 §2.1
    /// records as (16,72); the travel is the track width minus the thumb width.
    pub fn position_thumb_at(&self, position: u8) -> Rect {
        let position = position.min(100);
        let travel = self.position_track.width - self.position_thumb.width;
        let offset = (u32::from(position) * travel + 50) / 100;
        Rect::new(
            self.position_track.x + offset as i32,
            self.position_thumb.y,
            self.position_thumb.width,
            self.position_thumb.height,
        )
    }

    /// The volume thumb rectangle for a 0..=100 value.
    ///
    /// **Provisional** (S01 open question 4): S01 records that at value 0 the
    /// thumb's left edge sits at window x ~= 108 but not the exact blit origin.
    pub fn volume_thumb_at(&self, value: u8) -> Rect {
        let value = value.min(100);
        // One pixel of inset at value 0, as S01's measurement suggests.
        let travel = self.volume_track.width - VOLUME_THUMB_SIZE.0 - 1;
        let offset = (u32::from(value) * travel + 50) / 100;
        Rect::new(
            self.volume_track.x + 1 + offset as i32,
            self.volume_track.y + 1,
            VOLUME_THUMB_SIZE.0,
            VOLUME_THUMB_SIZE.1,
        )
    }

    /// The balance thumb rectangle for a 0..=100 value (50 = centred).
    ///
    /// **Provisional** (S01 open question 4): S01 does not give the balance
    /// thumb's origin.
    pub fn balance_thumb_at(&self, value: u8) -> Rect {
        let value = value.min(100);
        let travel = self.balance_track.width - BALANCE_THUMB_SIZE.0;
        let offset = (u32::from(value) * travel + 50) / 100;
        Rect::new(
            self.balance_track.x + offset as i32,
            self.balance_track.y + 1,
            BALANCE_THUMB_SIZE.0,
            BALANCE_THUMB_SIZE.1,
        )
    }
}

/// The volume/balance thumb size, 14 x 11 (S01 §2.1/open question 4,
/// provisional).
pub const VOLUME_THUMB_SIZE: (u32, u32) = (14, 11);
/// The balance thumb size, 14 x 11 (S01 §2.1/open question 4, provisional).
pub const BALANCE_THUMB_SIZE: (u32, u32) = (14, 11);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_sizes_match_s01() {
        assert_eq!((WINDOW_WIDTH, WINDOW_HEIGHT), (275, 116));
        assert_eq!((DOUBLE_WIDTH, DOUBLE_HEIGHT), (550, 232));
        assert_eq!(SHADE_HEIGHT, 14);
        assert_eq!(MainWindowLayout::window(), Rect::new(0, 0, 275, 116));
        assert_eq!(MainWindowLayout::shade_window(), Rect::new(0, 0, 275, 14));
    }

    /// Table-driven: one named case per S01 value, so a drift fails one name.
    #[test]
    fn every_layout_rectangle_matches_s01() {
        let l = MainWindowLayout::CLASSIC;
        let cases: [(&str, Rect, Rect); 31] = [
            ("background", l.background, Rect::new(0, 0, 275, 116)),
            ("title_bar", l.title_bar, Rect::new(0, 0, 275, 14)),
            (
                "shade_title_bar",
                l.shade_title_bar,
                Rect::new(0, 0, 275, 14),
            ),
            ("clutter_bar", l.clutter_bar, Rect::new(10, 22, 8, 43)),
            ("clutter_o", l.clutter_o, Rect::new(14, 25, 4, 6)),
            ("clutter_a", l.clutter_a, Rect::new(14, 33, 4, 6)),
            ("clutter_i", l.clutter_i, Rect::new(14, 41, 3, 6)),
            ("clutter_d", l.clutter_d, Rect::new(14, 49, 4, 6)),
            ("clutter_v", l.clutter_v, Rect::new(14, 56, 4, 6)),
            ("previous", l.previous, Rect::new(16, 88, 22, 18)),
            ("play", l.play, Rect::new(39, 88, 22, 18)),
            ("pause", l.pause, Rect::new(62, 88, 22, 18)),
            ("stop", l.stop, Rect::new(85, 88, 22, 18)),
            ("next", l.next, Rect::new(108, 88, 22, 18)),
            ("eject", l.eject, Rect::new(136, 89, 22, 16)),
            ("shuffle", l.shuffle, Rect::new(164, 89, 47, 15)),
            ("repeat", l.repeat, Rect::new(210, 89, 28, 15)),
            ("eq_toggle", l.eq_toggle, Rect::new(219, 58, 23, 12)),
            (
                "playlist_toggle",
                l.playlist_toggle,
                Rect::new(242, 58, 23, 12),
            ),
            (
                "position_track",
                l.position_track,
                Rect::new(16, 72, 248, 10),
            ),
            (
                "position_thumb",
                l.position_thumb,
                Rect::new(16, 72, 29, 10),
            ),
            ("volume_track", l.volume_track, Rect::new(107, 57, 68, 13)),
            ("balance_track", l.balance_track, Rect::new(177, 57, 38, 13)),
            ("time_digit_0", l.time_digits[0], Rect::new(48, 26, 9, 13)),
            ("time_digit_1", l.time_digits[1], Rect::new(60, 26, 9, 13)),
            ("time_digit_2", l.time_digits[2], Rect::new(78, 26, 9, 13)),
            ("time_digit_3", l.time_digits[3], Rect::new(90, 26, 9, 13)),
            ("minus_sign", l.minus_sign, Rect::new(38, 32, 5, 1)),
            ("play_indicator", l.play_indicator, Rect::new(26, 28, 9, 9)),
            (
                "decoder_indicator",
                l.decoder_indicator,
                Rect::new(24, 28, 3, 9),
            ),
            ("stereo", l.stereo, Rect::new(239, 41, 29, 12)),
        ];
        for (name, got, expected) in cases {
            assert_eq!(got, expected, "layout rectangle {name}");
        }
        assert_eq!(l.mono, Rect::new(212, 41, 27, 12));
        assert_eq!(l.time_display, Rect::new(36, 26, 60, 13));
        assert_eq!(l.marquee, Rect::new(111, 27, 154, 6));
        assert_eq!(l.kbps, Rect::new(111, 43, 15, 6));
        assert_eq!(l.khz, Rect::new(156, 43, 10, 6));
        assert_eq!(l.shade_position, Rect::new(226, 4, 17, 7));
        // S02 §7 window buttons: the sheet cells are verified, the window
        // positions are provisional (webamp cross-check).
        assert_eq!(l.window_button(WindowButton::Menu), Rect::new(6, 3, 9, 9));
        assert_eq!(
            l.window_button(WindowButton::Minimize),
            Rect::new(244, 3, 9, 9)
        );
        assert_eq!(
            l.window_button(WindowButton::Shade),
            Rect::new(254, 3, 9, 9)
        );
        assert_eq!(
            l.window_button(WindowButton::Close),
            Rect::new(264, 3, 9, 9)
        );
    }

    #[test]
    fn position_thumb_travels_across_the_track() {
        let l = MainWindowLayout::CLASSIC;
        assert_eq!(l.position_thumb_at(0), l.position_thumb);
        assert_eq!(l.position_thumb_at(0).x, 16);
        assert_eq!(l.position_thumb_at(100).x, 16 + (248 - 29));
        // The thumb never leaves the track.
        for p in 0..=100u8 {
            let thumb = l.position_thumb_at(p);
            assert!(thumb.x >= l.position_track.x);
            assert!(thumb.right() <= l.position_track.right());
        }
    }

    #[test]
    fn volume_and_balance_thumbs_stay_on_their_tracks() {
        let l = MainWindowLayout::CLASSIC;
        assert_eq!(l.volume_thumb_at(0).x, 108);
        for v in 0..=100u8 {
            let thumb = l.volume_thumb_at(v);
            assert!(thumb.x >= l.volume_track.x);
            assert!(thumb.right() <= l.volume_track.right());
            let balance = l.balance_thumb_at(v);
            assert!(balance.x >= l.balance_track.x);
            assert!(balance.right() <= l.balance_track.right());
        }
        // Centred balance is the middle of the track.
        let centred = l.balance_thumb_at(50);
        assert_eq!(centred.x + centred.width as i32 / 2, l.balance_track.x + 19);
    }
}
