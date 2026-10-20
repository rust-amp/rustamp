// MIT License
//
// Copyright (c) 2018 Jordan Eldredge
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Sprite tables for the classic skin sheets.
//!
//! The rectangles were originally ported from webamp (MIT), commit
//! `79117c66d79fdac690fc732a0ce8df543ccd97a0`, files
//! `packages/webamp/js/skinSprites.ts`, `packages/webamp/js/constants.ts` and
//! `packages/webamp/js/baseSkin.json`. They have since been **reconciled with
//! spec S01** (window geometry, audited 2026-10-14): every main-window row that
//! S01 measured carries S01's value and cites the spec section, and rows S01
//! left unconfirmed keep an explicit `provisional` marker.
//!
//! Where S01 and webamp disagree, **S01 wins**. S01's "webamp deltas" table
//! (S01 §2.4) lists exactly where the two differ; the changes applied here are:
//!
//! * transport buttons are **22 px wide at a 23 px pitch** (webamp used 23 px
//!   wide, which includes the separator column) — S01 §2.1/§2.4;
//! * the working/idle decoder-indicator sprites are **swapped** relative to
//!   webamp and are **3 x 9** — S01 §2.1/§2.4;
//! * the clutter-bar letter sprites are **4 x 6** (3 x 6 for the third letter),
//!   not webamp's 8 x 7 with transparent padding — S01 §2.1/§2.4;
//! * the position-slider thumb starts at the **measured x = 246**, not webamp's
//!   248; S01 records the residual as unresolved, so the row stays provisional
//!   — S01 §2.4/open question 5;
//! * the volume/balance thumb strip begins at **y = 420**, not webamp's 422 —
//!   S01 §2.4;
//! * `kbps`/`kHz` are drawn from **`text.bmp`**, not `numbers.bmp`; that is a
//!   rendering rule, not a rectangle — S01 §2.2/§2.4.
//!
//! Rows that S01 does not cover (the equalizer, playlist and generic-plugin
//! windows) are **unchanged** and stay marked `provisional: webamp`: their
//! windows belong to later cards, and S01 itself leaves the equalizer internals
//! (§7) and the playlist top half (§4) unresolved.
//!
//! See `THIRD_PARTY.md` for the attribution entry.

/// A sprite rectangle within a sprite sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteDef {
    /// The sprite name (webamp's identifier, upper snake case).
    pub name: &'static str,
    /// The sheet the rectangle lives in.
    pub sheet: Sheet,
    /// Left edge within the sheet.
    pub x: u32,
    /// Top edge within the sheet.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// A classic skin sprite sheet (one `.bmp` file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sheet {
    /// `balance.bmp`
    Balance,
    /// `cbuttons.bmp`
    Cbuttons,
    /// `main.bmp`
    Main,
    /// `monoster.bmp`
    Monoster,
    /// `numbers.bmp` — the classic time-display digits. A skin that carries
    /// `nums_ex.bmp` uses that one instead (S17 §3).
    Numbers,
    /// `nums_ex.bmp` — the optional extended digit sheet. When a skin carries
    /// it, the time display uses it **instead of** `numbers.bmp`, and the
    /// digit layout changes with it (S17 §3). The exact layout is still open
    /// (S01 question 6), so the rectangles below stay provisional.
    NumsEx,
    /// `playpaus.bmp`
    Playpaus,
    /// `pledit.bmp`
    Pledit,
    /// `eq_ex.bmp`
    EqEx,
    /// `eqmain.bmp`
    EqMain,
    /// `posbar.bmp`
    Posbar,
    /// `shufrep.bmp`
    Shufrep,
    /// `text.bmp`
    Text,
    /// `titlebar.bmp`
    Titlebar,
    /// `volume.bmp`
    Volume,
    /// `gen.bmp`
    Gen,
    /// `genex.bmp`
    GenEx,
    /// `mb.bmp`
    Mb,
    /// `video.bmp`
    Video,
}

impl Sheet {
    /// All sheets, in the order the loader reads them.
    pub const ALL: &'static [Sheet] = &[
        Sheet::Main,
        Sheet::Titlebar,
        Sheet::Cbuttons,
        Sheet::Shufrep,
        Sheet::Posbar,
        Sheet::Volume,
        Sheet::Balance,
        Sheet::Monoster,
        Sheet::Playpaus,
        Sheet::Numbers,
        Sheet::NumsEx,
        Sheet::Text,
        Sheet::Pledit,
        Sheet::EqMain,
        Sheet::EqEx,
        Sheet::Gen,
        Sheet::GenEx,
        Sheet::Mb,
        Sheet::Video,
    ];

    /// The file name this sheet is read from.
    pub const fn file_name(self) -> &'static str {
        match self {
            Sheet::Balance => "balance.bmp",
            Sheet::Cbuttons => "cbuttons.bmp",
            Sheet::Main => "main.bmp",
            Sheet::Monoster => "monoster.bmp",
            Sheet::Numbers => "numbers.bmp",
            Sheet::NumsEx => "nums_ex.bmp",
            Sheet::Playpaus => "playpaus.bmp",
            Sheet::Pledit => "pledit.bmp",
            Sheet::EqEx => "eq_ex.bmp",
            Sheet::EqMain => "eqmain.bmp",
            Sheet::Posbar => "posbar.bmp",
            Sheet::Shufrep => "shufrep.bmp",
            Sheet::Text => "text.bmp",
            Sheet::Titlebar => "titlebar.bmp",
            Sheet::Volume => "volume.bmp",
            Sheet::Gen => "gen.bmp",
            Sheet::GenEx => "genex.bmp",
            Sheet::Mb => "mb.bmp",
            Sheet::Video => "video.bmp",
        }
    }

    /// The sheet's file name without the `.bmp` extension, as used to look it
    /// up in a skin container.
    pub const fn stem(self) -> &'static str {
        match self {
            Sheet::Balance => "balance",
            Sheet::Cbuttons => "cbuttons",
            Sheet::Main => "main",
            Sheet::Monoster => "monoster",
            Sheet::Numbers => "numbers",
            Sheet::NumsEx => "nums_ex",
            Sheet::Playpaus => "playpaus",
            Sheet::Pledit => "pledit",
            Sheet::EqEx => "eq_ex",
            Sheet::EqMain => "eqmain",
            Sheet::Posbar => "posbar",
            Sheet::Shufrep => "shufrep",
            Sheet::Text => "text",
            Sheet::Titlebar => "titlebar",
            Sheet::Volume => "volume",
            Sheet::Gen => "gen",
            Sheet::GenEx => "genex",
            Sheet::Mb => "mb",
            Sheet::Video => "video",
        }
    }

    /// The sheets the main window needs to render.
    pub const fn required_main_window() -> &'static [Sheet] {
        &[
            Sheet::Main,
            Sheet::Titlebar,
            Sheet::Cbuttons,
            Sheet::Shufrep,
            Sheet::Posbar,
            Sheet::Volume,
            Sheet::Balance,
            Sheet::Monoster,
            Sheet::Playpaus,
            Sheet::Numbers,
            Sheet::Text,
        ]
    }
}

const fn def(
    name: &'static str,
    sheet: Sheet,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> SpriteDef {
    SpriteDef {
        name,
        sheet,
        x,
        y,
        width,
        height,
    }
}

/// Every named sprite rectangle.
///
/// Main-window rows cite S01 and carry S01's value; rows S01 marks provisional
/// keep that marker. Equalizer, playlist and generic-plugin rows are unchanged
/// from webamp and marked `provisional: webamp`.
#[rustfmt::skip]
pub static SPRITES: &[SpriteDef] = &[
    // --- balance.bmp (S01 open question 4: frame-to-value mapping unresolved) ---
    def("MAIN_BALANCE_BACKGROUND", Sheet::Balance, 9, 0, 38, 420), // S01 §2.4 provisional: 28 frames of 15 rows, mapping unresolved (open question 4)
    def("MAIN_BALANCE_THUMB", Sheet::Balance, 15, 420, 14, 11), // S01 §2.4 provisional: thumb strip starts at y=420; mapping unresolved (open question 4)
    def("MAIN_BALANCE_THUMB_ACTIVE", Sheet::Balance, 0, 420, 14, 11), // S01 §2.4 provisional: thumb strip starts at y=420; mapping unresolved (open question 4)
    // --- cbuttons.bmp ---
    def("MAIN_PREVIOUS_BUTTON", Sheet::Cbuttons, 0, 0, 22, 18), // S01 §2.1 verified (22 px wide at pitch 23)
    def("MAIN_PREVIOUS_BUTTON_ACTIVE", Sheet::Cbuttons, 0, 18, 22, 18), // S01 §2.1 provisional (pressed states never captured)
    def("MAIN_PLAY_BUTTON", Sheet::Cbuttons, 23, 0, 22, 18), // S01 §2.1 verified (22 px wide at pitch 23)
    def("MAIN_PLAY_BUTTON_ACTIVE", Sheet::Cbuttons, 23, 18, 22, 18), // S01 §2.1 provisional (pressed states never captured)
    def("MAIN_PAUSE_BUTTON", Sheet::Cbuttons, 46, 0, 22, 18), // S01 §2.1 verified (22 px wide at pitch 23)
    def("MAIN_PAUSE_BUTTON_ACTIVE", Sheet::Cbuttons, 46, 18, 22, 18), // S01 §2.1 provisional (pressed states never captured)
    def("MAIN_STOP_BUTTON", Sheet::Cbuttons, 69, 0, 22, 18), // S01 §2.1 verified (22 px wide at pitch 23)
    def("MAIN_STOP_BUTTON_ACTIVE", Sheet::Cbuttons, 69, 18, 22, 18), // S01 §2.1 provisional (pressed states never captured)
    def("MAIN_NEXT_BUTTON", Sheet::Cbuttons, 92, 0, 22, 18), // S01 §2.1 verified (22 px wide at pitch 23)
    def("MAIN_NEXT_BUTTON_ACTIVE", Sheet::Cbuttons, 92, 18, 22, 18), // S01 §2.1/§2.4 verified
    def("MAIN_EJECT_BUTTON", Sheet::Cbuttons, 114, 0, 22, 16), // S01 §2.1 verified
    def("MAIN_EJECT_BUTTON_ACTIVE", Sheet::Cbuttons, 114, 16, 22, 16), // S01 §2.1 provisional (pressed states never captured)
    // --- main.bmp ---
    def("MAIN_WINDOW_BACKGROUND", Sheet::Main, 0, 0, 275, 116), // S01 §2.1 verified
    // --- monoster.bmp ---
    def("MAIN_STEREO", Sheet::Monoster, 0, 12, 29, 12), // S01 §2.1 verified (dim)
    def("MAIN_STEREO_SELECTED", Sheet::Monoster, 0, 0, 29, 12), // S01 §2.1 verified (lit)
    def("MAIN_MONO", Sheet::Monoster, 29, 12, 27, 12), // S01 §2.1 verified (dim)
    def("MAIN_MONO_SELECTED", Sheet::Monoster, 29, 0, 27, 12), // S01 §2.1 verified (lit)
    // --- numbers.bmp (S01 §2.3) ---
    def("NO_MINUS_SIGN", Sheet::Numbers, 9, 6, 5, 1), // S01 §2.3 verified (dithered "no minus")
    def("MINUS_SIGN", Sheet::Numbers, 20, 6, 5, 1), // S01 §2.3 verified
    def("DIGIT_0", Sheet::Numbers, 0, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_1", Sheet::Numbers, 9, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_2", Sheet::Numbers, 18, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_3", Sheet::Numbers, 27, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_4", Sheet::Numbers, 36, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_5", Sheet::Numbers, 45, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_6", Sheet::Numbers, 54, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_7", Sheet::Numbers, 63, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_8", Sheet::Numbers, 72, 0, 9, 13), // S01 §2.3 verified
    def("DIGIT_9", Sheet::Numbers, 81, 0, 9, 13), // S01 §2.3 verified
    // The 11th 9x13 cell is the all-segments-off "no digit" filler used for a
    // leading blank position (S01 §2.3). webamp has no sprite for it.
    def("NO_DIGIT", Sheet::Numbers, 90, 0, 9, 13), // S01 §2.3 verified (cell 10, filler/"no digit")
    // --- nums_ex.bmp (the optional variant; layout unresolved) ---
    def("NO_MINUS_SIGN_EX", Sheet::NumsEx, 90, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("MINUS_SIGN_EX", Sheet::NumsEx, 99, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    // The nums_ex "no digit" cell is webamp's NO_MINUS_SIGN_EX (90,0): the
    // 9x13 blank next to the minus sign. The exact nums_ex layout is unresolved
    // (S01 open question 6, S17 §3).
    def("DIGIT_0_EX", Sheet::NumsEx, 0, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_1_EX", Sheet::NumsEx, 9, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_2_EX", Sheet::NumsEx, 18, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_3_EX", Sheet::NumsEx, 27, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_4_EX", Sheet::NumsEx, 36, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_5_EX", Sheet::NumsEx, 45, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_6_EX", Sheet::NumsEx, 54, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_7_EX", Sheet::NumsEx, 63, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_8_EX", Sheet::NumsEx, 72, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    def("DIGIT_9_EX", Sheet::NumsEx, 81, 0, 9, 13), // provisional: webamp; S01 open question 6, S17 §3
    // --- playpaus.bmp ---
    def("MAIN_PLAYING_INDICATOR", Sheet::Playpaus, 0, 0, 9, 9), // S01 §2.1 verified
    def("MAIN_PAUSED_INDICATOR", Sheet::Playpaus, 9, 0, 9, 9), // S01 §2.1 verified
    def("MAIN_STOPPED_INDICATOR", Sheet::Playpaus, 18, 0, 9, 9), // S01 §2.1 verified
    // S01 §2.4: the decoder-active sprite is (36,0,3,9); (39,0,3,9) is the idle
    // one. webamp had the two swapped and both 9 px wide.
    def("MAIN_WORKING_INDICATOR", Sheet::Playpaus, 36, 0, 3, 9), // S01 §2.1/§2.4 verified (decoder active)
    def("MAIN_NOT_WORKING_INDICATOR", Sheet::Playpaus, 39, 0, 3, 9), // S01 §2.1/§2.4 verified (idle)
    // --- pledit.bmp (playlist editor; unchanged from webamp, later card) ---
    def("PLAYLIST_TOP_TILE", Sheet::Pledit, 127, 21, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TOP_LEFT_CORNER", Sheet::Pledit, 0, 21, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TITLE_BAR", Sheet::Pledit, 26, 21, 100, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TOP_RIGHT_CORNER", Sheet::Pledit, 153, 21, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TOP_TILE_SELECTED", Sheet::Pledit, 127, 0, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TOP_LEFT_SELECTED", Sheet::Pledit, 0, 0, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TITLE_BAR_SELECTED", Sheet::Pledit, 26, 0, 100, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_TOP_RIGHT_CORNER_SELECTED", Sheet::Pledit, 153, 0, 25, 20), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_LEFT_TILE", Sheet::Pledit, 0, 42, 12, 29), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_RIGHT_TILE", Sheet::Pledit, 31, 42, 20, 29), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_BOTTOM_TILE", Sheet::Pledit, 179, 0, 25, 38), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_BOTTOM_LEFT_CORNER", Sheet::Pledit, 0, 72, 125, 38), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_BOTTOM_RIGHT_CORNER", Sheet::Pledit, 126, 72, 150, 38), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_VISUALIZER_BACKGROUND", Sheet::Pledit, 205, 0, 75, 38), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SHADE_BACKGROUND", Sheet::Pledit, 72, 57, 25, 14), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SHADE_BACKGROUND_LEFT", Sheet::Pledit, 72, 42, 25, 14), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SHADE_BACKGROUND_RIGHT", Sheet::Pledit, 99, 57, 50, 14), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SHADE_BACKGROUND_RIGHT_SELECTED", Sheet::Pledit, 99, 42, 50, 14), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SCROLL_HANDLE_SELECTED", Sheet::Pledit, 61, 53, 8, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SCROLL_HANDLE", Sheet::Pledit, 52, 53, 8, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_URL", Sheet::Pledit, 0, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_URL_SELECTED", Sheet::Pledit, 23, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_DIR", Sheet::Pledit, 0, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_DIR_SELECTED", Sheet::Pledit, 23, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_FILE", Sheet::Pledit, 0, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_FILE_SELECTED", Sheet::Pledit, 23, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_ALL", Sheet::Pledit, 54, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_ALL_SELECTED", Sheet::Pledit, 77, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_CROP", Sheet::Pledit, 54, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_CROP_SELECTED", Sheet::Pledit, 77, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_SELECTED", Sheet::Pledit, 54, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_SELECTED_SELECTED", Sheet::Pledit, 77, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_MISC", Sheet::Pledit, 54, 168, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_MISC_SELECTED", Sheet::Pledit, 77, 168, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_INVERT_SELECTION", Sheet::Pledit, 104, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_INVERT_SELECTION_SELECTED", Sheet::Pledit, 127, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SELECT_ZERO", Sheet::Pledit, 104, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SELECT_ZERO_SELECTED", Sheet::Pledit, 127, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SELECT_ALL", Sheet::Pledit, 104, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SELECT_ALL_SELECTED", Sheet::Pledit, 127, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SORT_LIST", Sheet::Pledit, 154, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SORT_LIST_SELECTED", Sheet::Pledit, 177, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_FILE_INFO", Sheet::Pledit, 154, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_FILE_INFO_SELECTED", Sheet::Pledit, 177, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_MISC_OPTIONS", Sheet::Pledit, 154, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_MISC_OPTIONS_SELECTED", Sheet::Pledit, 177, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_NEW_LIST", Sheet::Pledit, 204, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_NEW_LIST_SELECTED", Sheet::Pledit, 227, 111, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SAVE_LIST", Sheet::Pledit, 204, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SAVE_LIST_SELECTED", Sheet::Pledit, 227, 130, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_LOAD_LIST", Sheet::Pledit, 204, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_LOAD_LIST_SELECTED", Sheet::Pledit, 227, 149, 22, 18), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_ADD_MENU_BAR", Sheet::Pledit, 48, 111, 3, 54), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_REMOVE_MENU_BAR", Sheet::Pledit, 100, 111, 3, 72), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_SELECT_MENU_BAR", Sheet::Pledit, 150, 111, 3, 54), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_MISC_MENU_BAR", Sheet::Pledit, 200, 111, 3, 54), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_LIST_BAR", Sheet::Pledit, 250, 111, 3, 54), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_CLOSE_SELECTED", Sheet::Pledit, 52, 42, 9, 9), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_COLLAPSE_SELECTED", Sheet::Pledit, 62, 42, 9, 9), // provisional: webamp; S01 §4 provisional
    def("PLAYLIST_EXPAND_SELECTED", Sheet::Pledit, 150, 42, 9, 9), // provisional: webamp; S01 §4 provisional
    // --- eq_ex.bmp (equalizer shade; S01 §3/§7 provisional, later card) ---
    def("EQ_SHADE_BACKGROUND_SELECTED", Sheet::EqEx, 0, 0, 275, 14), // provisional: webamp; S01 §3 provisional
    def("EQ_SHADE_BACKGROUND", Sheet::EqEx, 0, 15, 275, 14), // provisional: webamp; S01 §3 provisional
    def("EQ_SHADE_VOLUME_SLIDER_LEFT", Sheet::EqEx, 1, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_SHADE_VOLUME_SLIDER_CENTER", Sheet::EqEx, 4, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_SHADE_VOLUME_SLIDER_RIGHT", Sheet::EqEx, 7, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_SHADE_BALANCE_SLIDER_LEFT", Sheet::EqEx, 11, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_SHADE_BALANCE_SLIDER_CENTER", Sheet::EqEx, 14, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_SHADE_BALANCE_SLIDER_RIGHT", Sheet::EqEx, 17, 30, 3, 7), // provisional: webamp; not in S01
    def("EQ_MAXIMIZE_BUTTON_ACTIVE", Sheet::EqEx, 1, 38, 9, 9), // provisional: webamp; not in S01
    def("EQ_MINIMIZE_BUTTON_ACTIVE", Sheet::EqEx, 1, 47, 9, 9), // provisional: webamp; not in S01
    def("EQ_SHADE_CLOSE_BUTTON", Sheet::EqEx, 11, 38, 9, 9), // provisional: webamp; not in S01
    def("EQ_SHADE_CLOSE_BUTTON_ACTIVE", Sheet::EqEx, 11, 47, 9, 9), // provisional: webamp; not in S01
    // --- eqmain.bmp (S01 §3/§7 provisional, later card) ---
    def("EQ_WINDOW_BACKGROUND", Sheet::EqMain, 0, 0, 275, 116), // S01 §3 verified (base layer)
    def("EQ_TITLE_BAR", Sheet::EqMain, 0, 149, 275, 14), // S01 §3 verified (drawn in the capture set)
    def("EQ_TITLE_BAR_SELECTED", Sheet::EqMain, 0, 134, 275, 14), // S01 §3 provisional (focus mapping unresolved)
    def("EQ_SLIDER_BACKGROUND", Sheet::EqMain, 13, 164, 209, 129), // S01 §3 provisional
    def("EQ_SLIDER_THUMB", Sheet::EqMain, 0, 164, 11, 11), // provisional: webamp; S01 §3 lists a different rect
    def("EQ_SLIDER_THUMB_SELECTED", Sheet::EqMain, 0, 176, 11, 11), // provisional: webamp; S01 §3 lists a different rect
    def("EQ_CLOSE_BUTTON", Sheet::EqMain, 0, 116, 9, 9), // provisional: webamp; not in S01
    def("EQ_CLOSE_BUTTON_ACTIVE", Sheet::EqMain, 0, 125, 9, 9), // provisional: webamp; not in S01
    def("EQ_MAXIMIZE_BUTTON_ACTIVE_FALLBACK", Sheet::EqMain, 254, 152, 9, 9), // provisional: webamp; not in S01
    def("EQ_ON_BUTTON", Sheet::EqMain, 10, 119, 25, 12), // S01 §3 round 2
    def("EQ_ON_BUTTON_DEPRESSED", Sheet::EqMain, 128, 119, 25, 12), // S01 §3 held-states run
    def("EQ_ON_BUTTON_SELECTED", Sheet::EqMain, 69, 119, 25, 12), // S01 §3 round 2
    def("EQ_ON_BUTTON_SELECTED_DEPRESSED", Sheet::EqMain, 187, 119, 25, 12), // S01 §3 held-states run
    def("EQ_AUTO_BUTTON", Sheet::EqMain, 36, 119, 32, 12), // S01 §3 verified
    def("EQ_AUTO_BUTTON_DEPRESSED", Sheet::EqMain, 153, 119, 33, 12), // S01 §3 held-states run
    def("EQ_AUTO_BUTTON_SELECTED", Sheet::EqMain, 95, 119, 32, 12), // S01 §3 provisional (engaged)
    def("EQ_AUTO_BUTTON_SELECTED_DEPRESSED", Sheet::EqMain, 212, 119, 33, 12), // S01 §3 held-states run
    def("EQ_GRAPH_BACKGROUND", Sheet::EqMain, 0, 294, 113, 19), // S01 §3 provisional (composite with the drawn curve)
    def("EQ_GRAPH_LINE_COLORS", Sheet::EqMain, 115, 294, 1, 19), // provisional: webamp; not in S01
    def("EQ_PRESETS_BUTTON", Sheet::EqMain, 224, 164, 44, 12), // S01 §3 verified
    def("EQ_PRESETS_BUTTON_SELECTED", Sheet::EqMain, 224, 176, 44, 12), // S01 §3 provisional (engaged)
    def("EQ_PREAMP_LINE", Sheet::EqMain, 0, 314, 113, 1), // provisional: webamp; not in S01
    // --- posbar.bmp ---
    def("MAIN_POSITION_SLIDER_BACKGROUND", Sheet::Posbar, 0, 0, 248, 10), // S01 §2.1 verified
    // S01 §2.4: the thumb graphic starts at x = 246-248 and the measured best
    // fit is 2 px left of webamp's 248. The residual is unresolved, so the row
    // stays provisional (S01 open question 5).
    def("MAIN_POSITION_SLIDER_THUMB", Sheet::Posbar, 246, 0, 29, 10), // S01 §2.4 provisional: measured x=246, residual unresolved (open question 5)
    def("MAIN_POSITION_SLIDER_THUMB_SELECTED", Sheet::Posbar, 278, 0, 29, 10), // S01 §2.1 provisional: dragged variant, residual unresolved
    // --- shufrep.bmp ---
    def("MAIN_SHUFFLE_BUTTON", Sheet::Shufrep, 28, 0, 47, 15), // S01 §2.1 verified
    def("MAIN_SHUFFLE_BUTTON_DEPRESSED", Sheet::Shufrep, 28, 15, 47, 15), // provisional: webamp; S01 §2.1 lists only idle and lit
    def("MAIN_SHUFFLE_BUTTON_SELECTED", Sheet::Shufrep, 28, 30, 47, 15), // S01 §2.1 verified (lit)
    def("MAIN_SHUFFLE_BUTTON_SELECTED_DEPRESSED", Sheet::Shufrep, 28, 45, 47, 15), // provisional: webamp; not in S01
    def("MAIN_REPEAT_BUTTON", Sheet::Shufrep, 0, 0, 28, 15), // S01 §2.1 verified
    def("MAIN_REPEAT_BUTTON_DEPRESSED", Sheet::Shufrep, 0, 15, 28, 15), // provisional: webamp; S01 §2.1 lists only idle and lit
    def("MAIN_REPEAT_BUTTON_SELECTED", Sheet::Shufrep, 0, 30, 28, 15), // S01 §2.1 verified (lit)
    def("MAIN_REPEAT_BUTTON_SELECTED_DEPRESSED", Sheet::Shufrep, 0, 45, 28, 15), // provisional: webamp; not in S01
    def("MAIN_EQ_BUTTON", Sheet::Shufrep, 0, 61, 23, 12), // S01 §2.1 verified
    def("MAIN_EQ_BUTTON_SELECTED", Sheet::Shufrep, 0, 73, 23, 12), // S01 §2.1 verified (window open)
    def("MAIN_EQ_BUTTON_DEPRESSED", Sheet::Shufrep, 46, 61, 23, 12), // provisional: webamp; not in S01
    def("MAIN_EQ_BUTTON_DEPRESSED_SELECTED", Sheet::Shufrep, 46, 73, 23, 12), // provisional: webamp; not in S01
    def("MAIN_PLAYLIST_BUTTON", Sheet::Shufrep, 23, 61, 23, 12), // S01 §2.1 verified
    def("MAIN_PLAYLIST_BUTTON_SELECTED", Sheet::Shufrep, 23, 73, 23, 12), // S01 §2.1 verified (window open)
    def("MAIN_PLAYLIST_BUTTON_DEPRESSED", Sheet::Shufrep, 69, 61, 23, 12), // provisional: webamp; not in S01
    def("MAIN_PLAYLIST_BUTTON_DEPRESSED_SELECTED", Sheet::Shufrep, 69, 73, 23, 12), // provisional: webamp; not in S01
    // --- titlebar.bmp ---
    def("MAIN_TITLE_BAR", Sheet::Titlebar, 27, 15, 275, 14), // S01 §2.1 verified (not focused)
    def("MAIN_TITLE_BAR_SELECTED", Sheet::Titlebar, 27, 0, 275, 14), // S01 §2.1 verified (focused)
    def("MAIN_EASTER_EGG_TITLE_BAR", Sheet::Titlebar, 27, 72, 275, 14), // S01 §2.1 provisional
    def("MAIN_EASTER_EGG_TITLE_BAR_SELECTED", Sheet::Titlebar, 27, 57, 275, 14), // S01 §2.1 provisional
    // S01 §2.4/open question 1: 2.95 draws the window buttons from the title bar
    // body sprite itself; the 9x9 cells at x = 0-26 are not read. Kept so a
    // later card can still name them, but they are not blitted.
    def("MAIN_OPTIONS_BUTTON", Sheet::Titlebar, 0, 0, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_OPTIONS_BUTTON_DEPRESSED", Sheet::Titlebar, 0, 9, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_MINIMIZE_BUTTON", Sheet::Titlebar, 9, 0, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_MINIMIZE_BUTTON_DEPRESSED", Sheet::Titlebar, 9, 9, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_SHADE_BUTTON", Sheet::Titlebar, 0, 18, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_SHADE_BUTTON_DEPRESSED", Sheet::Titlebar, 9, 18, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_CLOSE_BUTTON", Sheet::Titlebar, 18, 0, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_CLOSE_BUTTON_DEPRESSED", Sheet::Titlebar, 18, 9, 9, 9), // S01 §2.4: not drawn by 2.95 (open question 1)
    def("MAIN_CLUTTER_BAR_BACKGROUND", Sheet::Titlebar, 304, 0, 8, 43), // S01 §2.1 verified
    def("MAIN_CLUTTER_BAR_BACKGROUND_DISABLED", Sheet::Titlebar, 312, 0, 8, 43), // provisional: webamp; not in S01
    // S01 §2.4: the drawn letter sprites are 4x6 (3x6 for the third letter) on a
    // diagonal in the sheet; webamp's rectangles include transparent padding.
    def("MAIN_CLUTTER_BAR_BUTTON_O_SELECTED", Sheet::Titlebar, 308, 47, 4, 6), // S01 §2.1 provisional (never captured lit)
    def("MAIN_CLUTTER_BAR_BUTTON_A_SELECTED", Sheet::Titlebar, 316, 55, 4, 6), // S01 §2.1 verified
    def("MAIN_CLUTTER_BAR_BUTTON_I_SELECTED", Sheet::Titlebar, 324, 63, 3, 6), // S01 §2.1 verified
    def("MAIN_CLUTTER_BAR_BUTTON_D_SELECTED", Sheet::Titlebar, 332, 71, 4, 6), // S01 §2.1 verified
    def("MAIN_CLUTTER_BAR_BUTTON_V_SELECTED", Sheet::Titlebar, 340, 78, 4, 6), // S01 §2.1 verified
    def("MAIN_SHADE_BACKGROUND", Sheet::Titlebar, 27, 42, 275, 14), // S01 §2.1 provisional (shade, not focused)
    def("MAIN_SHADE_BACKGROUND_SELECTED", Sheet::Titlebar, 27, 29, 275, 14), // S01 §2.1 provisional (shade, focused)
    def("MAIN_SHADE_BUTTON_SELECTED", Sheet::Titlebar, 0, 27, 9, 9), // provisional: webamp; not in S01
    def("MAIN_SHADE_BUTTON_SELECTED_DEPRESSED", Sheet::Titlebar, 9, 27, 9, 9), // provisional: webamp; not in S01
    def("MAIN_SHADE_POSITION_BACKGROUND", Sheet::Titlebar, 0, 36, 17, 7), // provisional: webamp; not in S01
    def("MAIN_SHADE_POSITION_THUMB", Sheet::Titlebar, 20, 36, 3, 7), // provisional: webamp; not in S01
    def("MAIN_SHADE_POSITION_THUMB_LEFT", Sheet::Titlebar, 17, 36, 3, 7), // provisional: webamp; not in S01
    def("MAIN_SHADE_POSITION_THUMB_RIGHT", Sheet::Titlebar, 23, 36, 3, 7), // provisional: webamp; not in S01
    // --- volume.bmp (S01 open question 4: frame-to-value mapping unresolved) ---
    def("MAIN_VOLUME_BACKGROUND", Sheet::Volume, 0, 0, 68, 420), // S01 §2.4 provisional: 28 frames of 15 rows, mapping unresolved (open question 4)
    def("MAIN_VOLUME_THUMB", Sheet::Volume, 15, 420, 14, 11), // S01 §2.4 provisional: thumb strip starts at y=420; mapping unresolved (open question 4)
    def("MAIN_VOLUME_THUMB_SELECTED", Sheet::Volume, 0, 420, 14, 11), // S01 §2.4 provisional: thumb strip starts at y=420; mapping unresolved (open question 4)
    // --- gen.bmp (generic-plugin window; S11, later card) ---
    def("GEN_TOP_LEFT_SELECTED", Sheet::Gen, 0, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_LEFT_END_SELECTED", Sheet::Gen, 26, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_CENTER_FILL_SELECTED", Sheet::Gen, 52, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_RIGHT_END_SELECTED", Sheet::Gen, 78, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_LEFT_RIGHT_FILL_SELECTED", Sheet::Gen, 104, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_RIGHT_SELECTED", Sheet::Gen, 130, 0, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_LEFT", Sheet::Gen, 0, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_LEFT_END", Sheet::Gen, 26, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_CENTER_FILL", Sheet::Gen, 52, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_RIGHT_END", Sheet::Gen, 78, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_LEFT_RIGHT_FILL", Sheet::Gen, 104, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_TOP_RIGHT", Sheet::Gen, 130, 21, 25, 20), // provisional: webamp; S11 out of scope
    def("GEN_BOTTOM_LEFT", Sheet::Gen, 0, 42, 125, 14), // provisional: webamp; S11 out of scope
    def("GEN_BOTTOM_RIGHT", Sheet::Gen, 0, 57, 125, 14), // provisional: webamp; S11 out of scope
    def("GEN_BOTTOM_FILL", Sheet::Gen, 127, 72, 25, 14), // provisional: webamp; S11 out of scope
    def("GEN_MIDDLE_LEFT", Sheet::Gen, 127, 42, 11, 29), // provisional: webamp; S11 out of scope
    def("GEN_MIDDLE_LEFT_BOTTOM", Sheet::Gen, 158, 42, 11, 24), // provisional: webamp; S11 out of scope
    def("GEN_MIDDLE_RIGHT", Sheet::Gen, 139, 42, 8, 29), // provisional: webamp; S11 out of scope
    def("GEN_MIDDLE_RIGHT_BOTTOM", Sheet::Gen, 170, 42, 8, 24), // provisional: webamp; S11 out of scope
    def("GEN_CLOSE_SELECTED", Sheet::Gen, 148, 42, 9, 9), // provisional: webamp; S11 out of scope
];

/// The `text.bmp` cell size, ported from webamp (`CHAR_X` x `CHAR_Y`).
pub const TEXT_CELL: (u32, u32) = (5, 6); // S01 §2.2 verified (31 x 3 grid of 5 x 6 cells)

/// The `text.bmp` glyph grid, ported from webamp's `FONT_LOOKUP`.
///
/// Each entry maps a character to its `(column, row)` cell. The grid is 31
/// columns x 3 rows of 5 x 6 px cells, matching S01 §2.2. The characters are
/// stored lowercase because the classic font has no lowercase: a lowercase
/// letter is rendered with its uppercase glyph (S01 §2.2).
///
/// The values were transposed relative to the original webamp port, which
/// stored `(row, column)` although the type documents `(column, row)`; S01 §2.2
/// gives the grid order (`A`-`Z` in row 0, the digits in row 1, `Å Ö Ä ? *` in
/// row 2) and that is what is stored here.
#[rustfmt::skip]
pub static TEXT_GLYPHS: &[(char, u32, u32)] = &[
    ('a', 0, 0), ('b', 1, 0), ('c', 2, 0), ('d', 3, 0), ('e', 4, 0), ('f', 5, 0), // S01 §2.2 row 0
    ('g', 6, 0), ('h', 7, 0), ('i', 8, 0), ('j', 9, 0), ('k', 10, 0), ('l', 11, 0), // S01 §2.2 row 0
    ('m', 12, 0), ('n', 13, 0), ('o', 14, 0), ('p', 15, 0), ('q', 16, 0), ('r', 17, 0), // S01 §2.2 row 0
    ('s', 18, 0), ('t', 19, 0), ('u', 20, 0), ('v', 21, 0), ('w', 22, 0), ('x', 23, 0), // S01 §2.2 row 0
    ('y', 24, 0), ('z', 25, 0), ('"', 26, 0), ('@', 27, 0), (' ', 30, 0), // S01 §2.2 row 0
    ('0', 0, 1), ('1', 1, 1), ('2', 2, 1), ('3', 3, 1), ('4', 4, 1), ('5', 5, 1), // S01 §2.2 row 1
    ('6', 6, 1), ('7', 7, 1), ('8', 8, 1), ('9', 9, 1), ('\u{2026}', 10, 1), // S01 §2.2 row 1
    ('.', 11, 1), (':', 12, 1), ('(', 13, 1), (')', 14, 1), ('-', 15, 1), // S01 §2.2 row 1
    ('\'', 16, 1), ('!', 17, 1), ('_', 18, 1), ('+', 19, 1), ('\\', 20, 1), // S01 §2.2 row 1
    ('/', 21, 1), ('[', 22, 1), (']', 23, 1), ('^', 24, 1), ('&', 25, 1), // S01 §2.2 row 1
    ('%', 26, 1), (',', 27, 1), ('=', 28, 1), ('$', 29, 1), ('#', 30, 1), // S01 §2.2 row 1
    ('Å', 0, 2), ('Ö', 1, 2), ('Ä', 2, 2), ('?', 3, 2), ('*', 4, 2), // S01 §2.2 row 2
    ('<', 22, 1), ('>', 23, 1), ('{', 22, 1), ('}', 23, 1), // webamp aliases, not in S01
];

/// The 24 default analyser colours.
///
/// S17 §6 lists the base skin's `viscolor.txt`: 24 entries, slot 0 the
/// analyser background, slot 1 the grid dots, slots 2-17 the spectrum
/// gradient, slots 18-22 the oscilloscope colours and slot 23 the peak dots.
/// These are the built-in defaults, so a skin without a `viscolor.txt` gets
/// the classic look.
pub static DEFAULT_VISCOLORS: [[u8; 3]; 24] = [
    [0, 0, 0],       // S17 §6 (analyser background)
    [24, 33, 41],    // S17 §6 (grid dots)
    [239, 49, 16],   // S17 §6 (spectrum, top)
    [206, 41, 16],   // S17 §6
    [214, 90, 0],    // S17 §6
    [214, 102, 0],   // S17 §6
    [214, 115, 0],   // S17 §6
    [198, 123, 8],   // S17 §6
    [222, 165, 24],  // S17 §6
    [214, 181, 33],  // S17 §6
    [189, 222, 41],  // S17 §6
    [148, 222, 33],  // S17 §6
    [41, 206, 16],   // S17 §6
    [50, 190, 16],   // S17 §6
    [57, 181, 16],   // S17 §6
    [49, 156, 8],    // S17 §6
    [41, 148, 0],    // S17 §6
    [24, 132, 8],    // S17 §6 (spectrum, bottom)
    [255, 255, 255], // S17 §6 (oscilloscope)
    [214, 214, 222], // S17 §6
    [181, 189, 189], // S17 §6
    [160, 170, 175], // S17 §6
    [148, 156, 165], // S17 §6
    [150, 150, 150], // S17 §6 (analyser peak dots)
];

/// Default playlist colour values.
///
/// S17 §5 lists the base skin's `pledit.txt` and records that a playlist
/// without a `pledit.txt` still renders green / white / black, so these are
/// the built-in defaults of the classic look. The observation covered the
/// three text colours; the selected-row background was not observed, so it
/// keeps the value webamp's base skin carries (the base skin's own file uses
/// `#0000C6`, see the table in S17 §5).
pub const DEFAULT_PLAYLIST_NORMAL: [u8; 3] = [0x00, 0xFF, 0x00]; // S17 §5 verified
/// Default currently-playing colour.
pub const DEFAULT_PLAYLIST_CURRENT: [u8; 3] = [0xFF, 0xFF, 0xFF]; // S17 §5 verified
/// Default playlist background.
pub const DEFAULT_PLAYLIST_NORMAL_BG: [u8; 3] = [0x00, 0x00, 0x00]; // S17 §5 verified
/// Default selected-entry background.
///
/// The base skin's own `pledit.txt` carries `SelectedBG=#0000C6` (S17 §5's
/// table); the observed default until now was webamp's `#0000FF`. The owner
/// chose to follow the base skin (2026-10-20), so the built-in default is the
/// value the reference's own file states.
pub const DEFAULT_PLAYLIST_SELECTED_BG: [u8; 3] = [0x00, 0x00, 0xC6]; // base skin pledit.txt, owner decision 2026-10-20
/// Default playlist font.
pub const DEFAULT_PLAYLIST_FONT: &str = "Arial"; // provisional: webamp; S17 §5 did not observe it

/// Looks up a sprite by name.
pub fn sprite(name: &str) -> Option<&'static SpriteDef> {
    SPRITES.iter().find(|s| s.name == name)
}

/// Iterates the sprites that live in `sheet`.
pub fn sprites_in_sheet(sheet: Sheet) -> impl Iterator<Item = &'static SpriteDef> {
    SPRITES.iter().filter(move |s| s.sheet == sheet)
}

/// The bounding box `(width, height)` of a sheet as implied by the sprite
/// table, used to size the generated default skin.
///
/// `text.bmp` has no sprite rectangles (its glyphs live in [`TEXT_GLYPHS`]);
/// its extent is derived from the glyph grid instead.
pub fn sheet_extent(sheet: Sheet) -> (u32, u32) {
    match sheet {
        Sheet::Balance => return (68, 433),
        Sheet::Monoster => return (58, 24),
        Sheet::EqEx => return (275, 82),
        Sheet::Gen => return (194, 109),
        Sheet::GenEx => return (130, 75),
        Sheet::Mb => return (234, 119),
        Sheet::Video => return (234, 119),
        _ => {}
    }
    if sheet == Sheet::Text {
        let mut columns = 0;
        let mut rows = 0;
        for (_, col, row) in TEXT_GLYPHS {
            columns = columns.max(col + 1);
            rows = rows.max(row + 1);
        }
        return (columns * TEXT_CELL.0, rows * TEXT_CELL.1);
    }
    let mut width = 0;
    let mut height = 0;
    for s in sprites_in_sheet(sheet) {
        width = width.max(s.x + s.width);
        height = height.max(s.y + s.height);
    }
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sprite_names_are_unique() {
        let mut seen = HashSet::new();
        for s in SPRITES {
            assert!(seen.insert(s.name), "duplicate sprite name {}", s.name);
        }
    }

    #[test]
    fn all_main_window_sheets_have_sprites() {
        for sheet in Sheet::required_main_window() {
            let (w, h) = sheet_extent(*sheet);
            assert!(w > 0 && h > 0, "no extent for {:?}", sheet);
        }
    }

    #[test]
    fn lookup_finds_known_and_rejects_unknown() {
        let s = sprite("MAIN_PLAY_BUTTON").unwrap();
        assert_eq!(s.sheet, Sheet::Cbuttons);
        // S01 §2.1/§2.4: 22 px wide at a 23 px pitch, not webamp's 23 px.
        assert_eq!((s.x, s.y, s.width, s.height), (23, 0, 22, 18));
        assert!(sprite("NOT_A_SPRITE").is_none());
    }

    #[test]
    fn nums_ex_has_the_extra_digit_cells() {
        for digit in 0..10 {
            assert!(
                sprite(&format!("DIGIT_{digit}_EX")).is_some(),
                "missing DIGIT_{digit}_EX"
            );
        }
        // The 11th cell is the full-size minus/blank that `numbers.bmp` lacks.
        assert!(sprite("NO_MINUS_SIGN_EX").is_some());
        assert!(sprite("MINUS_SIGN_EX").is_some());
        assert_eq!(sheet_extent(Sheet::Numbers), (99, 13));
        assert_eq!(sheet_extent(Sheet::NumsEx), (108, 13));
    }

    /// S01 §2.4: the transport buttons are 22 px wide at a 23 px pitch.
    #[test]
    fn transport_buttons_are_22_wide_at_pitch_23() {
        for (name, x) in [
            ("MAIN_PREVIOUS_BUTTON", 0),
            ("MAIN_PLAY_BUTTON", 23),
            ("MAIN_PAUSE_BUTTON", 46),
            ("MAIN_STOP_BUTTON", 69),
            ("MAIN_NEXT_BUTTON", 92),
        ] {
            let s = sprite(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!((s.x, s.width), (x, 22), "{name}");
            let pressed = sprite(&format!("{name}_ACTIVE")).unwrap();
            assert_eq!((pressed.x, pressed.width), (x, 22), "{name}_ACTIVE");
        }
        let eject = sprite("MAIN_EJECT_BUTTON").unwrap();
        assert_eq!(
            (eject.x, eject.y, eject.width, eject.height),
            (114, 0, 22, 16)
        );
    }

    /// S01 §2.4: the decoder-active sprite is (36,0,3,9), the idle one (39,0,3,9).
    #[test]
    fn decoder_indicator_sprites_are_swapped_and_three_px_wide() {
        let working = sprite("MAIN_WORKING_INDICATOR").unwrap();
        let idle = sprite("MAIN_NOT_WORKING_INDICATOR").unwrap();
        assert_eq!(
            (working.x, working.y, working.width, working.height),
            (36, 0, 3, 9)
        );
        assert_eq!((idle.x, idle.y, idle.width, idle.height), (39, 0, 3, 9));
    }

    /// S01 §2.4: the clutter-bar letters are 4 x 6 (3 x 6 for the third).
    #[test]
    fn clutter_bar_letters_have_no_transparent_padding() {
        for (name, x, y, w) in [
            ("MAIN_CLUTTER_BAR_BUTTON_O_SELECTED", 308, 47, 4),
            ("MAIN_CLUTTER_BAR_BUTTON_A_SELECTED", 316, 55, 4),
            ("MAIN_CLUTTER_BAR_BUTTON_I_SELECTED", 324, 63, 3),
            ("MAIN_CLUTTER_BAR_BUTTON_D_SELECTED", 332, 71, 4),
            ("MAIN_CLUTTER_BAR_BUTTON_V_SELECTED", 340, 78, 4),
        ] {
            let s = sprite(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!((s.x, s.y, s.width, s.height), (x, y, w, 6), "{name}");
        }
    }

    /// S01 §2.4: the position thumb starts at the measured x, the volume and
    /// balance thumb strips begin at y = 420.
    #[test]
    fn slider_thumb_rects_follow_s01() {
        let pos = sprite("MAIN_POSITION_SLIDER_THUMB").unwrap();
        assert_eq!((pos.x, pos.y, pos.width, pos.height), (246, 0, 29, 10));
        for name in [
            "MAIN_VOLUME_THUMB",
            "MAIN_VOLUME_THUMB_SELECTED",
            "MAIN_BALANCE_THUMB",
            "MAIN_BALANCE_THUMB_ACTIVE",
        ] {
            let s = sprite(name).unwrap();
            assert_eq!(s.y, 420, "{name} thumb strip should start at y = 420");
            assert_eq!((s.width, s.height), (14, 11), "{name}");
        }
    }

    /// S01 §2.2: the glyph grid is 31 columns x 3 rows of 5 x 6 cells.
    #[test]
    fn text_glyph_grid_matches_s01() {
        assert_eq!(TEXT_CELL, (5, 6));
        assert_eq!(sheet_extent(Sheet::Text), (155, 18));
        // Spot-check the S01 §2.2 ordering: A-Z in row 0, digits in row 1.
        assert_eq!(sprite_cell('a'), (0, 0));
        assert_eq!(sprite_cell('z'), (25, 0));
        assert_eq!(sprite_cell('"'), (26, 0));
        assert_eq!(sprite_cell('@'), (27, 0));
        assert_eq!(sprite_cell(' '), (30, 0));
        assert_eq!(sprite_cell('0'), (0, 1));
        assert_eq!(sprite_cell('9'), (9, 1));
        assert_eq!(sprite_cell('.'), (11, 1));
        assert_eq!(sprite_cell('#'), (30, 1));
        assert_eq!(sprite_cell('Å'), (0, 2));
        assert_eq!(sprite_cell('*'), (4, 2));
    }

    fn sprite_cell(ch: char) -> (u32, u32) {
        TEXT_GLYPHS
            .iter()
            .find(|(c, _, _)| *c == ch)
            .map(|(_, col, row)| (*col, *row))
            .unwrap_or_else(|| panic!("missing glyph {ch:?}"))
    }
}
