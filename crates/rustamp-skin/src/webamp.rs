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

//! Provisional sprite tables ported from webamp.
//!
//! Every value in this module is ported from webamp (MIT), commit
//! `79117c66d79fdac690fc732a0ce8df543ccd97a0`, files
//! `packages/webamp/js/skinSprites.ts`, `packages/webamp/js/constants.ts` and
//! `packages/webamp/js/baseSkin.json`. It is a **stopgap**: the real coordinates
//! and default values must be validated against Winamp 2.95 by spec **S01**
//! (geometry/sprite rects) and spec **S17** (skin-loading quirks). Each value
//! carries a `// provisional: webamp, pending S01/S17` marker so the table can be
//! replaced wholesale once the specs exist.
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
    /// `numbers.bmp`
    Numbers,
    /// `nums_ex.bmp`
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
        }
    }

    /// The sheet a missing sheet falls back to, if any.
    ///
    /// `nums_ex.bmp` is the optional 11-digit variant; when it is absent the
    /// classic `numbers.bmp` is used.
    pub const fn fallback(self) -> Option<Sheet> {
        match self {
            Sheet::NumsEx => Some(Sheet::Numbers),
            _ => None,
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

/// Every named sprite rectangle, ported from webamp's `skinSprites.ts`.
#[rustfmt::skip]
pub static SPRITES: &[SpriteDef] = &[
    // --- balance.bmp ---
    def("MAIN_BALANCE_BACKGROUND", Sheet::Balance, 9, 0, 38, 420), // provisional: webamp, pending S01/S17
    def("MAIN_BALANCE_THUMB", Sheet::Balance, 15, 422, 14, 11), // provisional: webamp, pending S01/S17
    def("MAIN_BALANCE_THUMB_ACTIVE", Sheet::Balance, 0, 422, 14, 11), // provisional: webamp, pending S01/S17
    // --- cbuttons.bmp ---
    def("MAIN_PREVIOUS_BUTTON", Sheet::Cbuttons, 0, 0, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_PREVIOUS_BUTTON_ACTIVE", Sheet::Cbuttons, 0, 18, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_PLAY_BUTTON", Sheet::Cbuttons, 23, 0, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_PLAY_BUTTON_ACTIVE", Sheet::Cbuttons, 23, 18, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_PAUSE_BUTTON", Sheet::Cbuttons, 46, 0, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_PAUSE_BUTTON_ACTIVE", Sheet::Cbuttons, 46, 18, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_STOP_BUTTON", Sheet::Cbuttons, 69, 0, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_STOP_BUTTON_ACTIVE", Sheet::Cbuttons, 69, 18, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_NEXT_BUTTON", Sheet::Cbuttons, 92, 0, 23, 18), // provisional: webamp, pending S01/S17
    def("MAIN_NEXT_BUTTON_ACTIVE", Sheet::Cbuttons, 92, 18, 22, 18), // provisional: webamp, pending S01/S17
    def("MAIN_EJECT_BUTTON", Sheet::Cbuttons, 114, 0, 22, 16), // provisional: webamp, pending S01/S17
    def("MAIN_EJECT_BUTTON_ACTIVE", Sheet::Cbuttons, 114, 16, 22, 16), // provisional: webamp, pending S01/S17
    // --- main.bmp ---
    def("MAIN_WINDOW_BACKGROUND", Sheet::Main, 0, 0, 275, 116), // provisional: webamp, pending S01/S17
    // --- monoster.bmp ---
    def("MAIN_STEREO", Sheet::Monoster, 0, 12, 29, 12), // provisional: webamp, pending S01/S17
    def("MAIN_STEREO_SELECTED", Sheet::Monoster, 0, 0, 29, 12), // provisional: webamp, pending S01/S17
    def("MAIN_MONO", Sheet::Monoster, 29, 12, 27, 12), // provisional: webamp, pending S01/S17
    def("MAIN_MONO_SELECTED", Sheet::Monoster, 29, 0, 27, 12), // provisional: webamp, pending S01/S17
    // --- numbers.bmp ---
    def("NO_MINUS_SIGN", Sheet::Numbers, 9, 6, 5, 1), // provisional: webamp, pending S01/S17
    def("MINUS_SIGN", Sheet::Numbers, 20, 6, 5, 1), // provisional: webamp, pending S01/S17
    def("DIGIT_0", Sheet::Numbers, 0, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_1", Sheet::Numbers, 9, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_2", Sheet::Numbers, 18, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_3", Sheet::Numbers, 27, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_4", Sheet::Numbers, 36, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_5", Sheet::Numbers, 45, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_6", Sheet::Numbers, 54, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_7", Sheet::Numbers, 63, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_8", Sheet::Numbers, 72, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_9", Sheet::Numbers, 81, 0, 9, 13), // provisional: webamp, pending S01/S17
    // --- nums_ex.bmp (the 11-cell variant) ---
    def("NO_MINUS_SIGN_EX", Sheet::NumsEx, 90, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("MINUS_SIGN_EX", Sheet::NumsEx, 99, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_0_EX", Sheet::NumsEx, 0, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_1_EX", Sheet::NumsEx, 9, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_2_EX", Sheet::NumsEx, 18, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_3_EX", Sheet::NumsEx, 27, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_4_EX", Sheet::NumsEx, 36, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_5_EX", Sheet::NumsEx, 45, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_6_EX", Sheet::NumsEx, 54, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_7_EX", Sheet::NumsEx, 63, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_8_EX", Sheet::NumsEx, 72, 0, 9, 13), // provisional: webamp, pending S01/S17
    def("DIGIT_9_EX", Sheet::NumsEx, 81, 0, 9, 13), // provisional: webamp, pending S01/S17
    // --- playpaus.bmp ---
    def("MAIN_PLAYING_INDICATOR", Sheet::Playpaus, 0, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_PAUSED_INDICATOR", Sheet::Playpaus, 9, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_STOPPED_INDICATOR", Sheet::Playpaus, 18, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_NOT_WORKING_INDICATOR", Sheet::Playpaus, 36, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_WORKING_INDICATOR", Sheet::Playpaus, 39, 0, 9, 9), // provisional: webamp, pending S01/S17
    // --- pledit.bmp ---
    def("PLAYLIST_TOP_TILE", Sheet::Pledit, 127, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TOP_LEFT_CORNER", Sheet::Pledit, 0, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TITLE_BAR", Sheet::Pledit, 26, 21, 100, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TOP_RIGHT_CORNER", Sheet::Pledit, 153, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TOP_TILE_SELECTED", Sheet::Pledit, 127, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TOP_LEFT_SELECTED", Sheet::Pledit, 0, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TITLE_BAR_SELECTED", Sheet::Pledit, 26, 0, 100, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_TOP_RIGHT_CORNER_SELECTED", Sheet::Pledit, 153, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("PLAYLIST_LEFT_TILE", Sheet::Pledit, 0, 42, 12, 29), // provisional: webamp, pending S01/S17
    def("PLAYLIST_RIGHT_TILE", Sheet::Pledit, 31, 42, 20, 29), // provisional: webamp, pending S01/S17
    def("PLAYLIST_BOTTOM_TILE", Sheet::Pledit, 179, 0, 25, 38), // provisional: webamp, pending S01/S17
    def("PLAYLIST_BOTTOM_LEFT_CORNER", Sheet::Pledit, 0, 72, 125, 38), // provisional: webamp, pending S01/S17
    def("PLAYLIST_BOTTOM_RIGHT_CORNER", Sheet::Pledit, 126, 72, 150, 38), // provisional: webamp, pending S01/S17
    def("PLAYLIST_VISUALIZER_BACKGROUND", Sheet::Pledit, 205, 0, 75, 38), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SHADE_BACKGROUND", Sheet::Pledit, 72, 57, 25, 14), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SHADE_BACKGROUND_LEFT", Sheet::Pledit, 72, 42, 25, 14), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SHADE_BACKGROUND_RIGHT", Sheet::Pledit, 99, 57, 50, 14), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SHADE_BACKGROUND_RIGHT_SELECTED", Sheet::Pledit, 99, 42, 50, 14), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SCROLL_HANDLE_SELECTED", Sheet::Pledit, 61, 53, 8, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SCROLL_HANDLE", Sheet::Pledit, 52, 53, 8, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_URL", Sheet::Pledit, 0, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_URL_SELECTED", Sheet::Pledit, 23, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_DIR", Sheet::Pledit, 0, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_DIR_SELECTED", Sheet::Pledit, 23, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_FILE", Sheet::Pledit, 0, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_FILE_SELECTED", Sheet::Pledit, 23, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_ALL", Sheet::Pledit, 54, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_ALL_SELECTED", Sheet::Pledit, 77, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_CROP", Sheet::Pledit, 54, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_CROP_SELECTED", Sheet::Pledit, 77, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_SELECTED", Sheet::Pledit, 54, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_SELECTED_SELECTED", Sheet::Pledit, 77, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_MISC", Sheet::Pledit, 54, 168, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_MISC_SELECTED", Sheet::Pledit, 77, 168, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_INVERT_SELECTION", Sheet::Pledit, 104, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_INVERT_SELECTION_SELECTED", Sheet::Pledit, 127, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SELECT_ZERO", Sheet::Pledit, 104, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SELECT_ZERO_SELECTED", Sheet::Pledit, 127, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SELECT_ALL", Sheet::Pledit, 104, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SELECT_ALL_SELECTED", Sheet::Pledit, 127, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SORT_LIST", Sheet::Pledit, 154, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SORT_LIST_SELECTED", Sheet::Pledit, 177, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_FILE_INFO", Sheet::Pledit, 154, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_FILE_INFO_SELECTED", Sheet::Pledit, 177, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_MISC_OPTIONS", Sheet::Pledit, 154, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_MISC_OPTIONS_SELECTED", Sheet::Pledit, 177, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_NEW_LIST", Sheet::Pledit, 204, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_NEW_LIST_SELECTED", Sheet::Pledit, 227, 111, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SAVE_LIST", Sheet::Pledit, 204, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SAVE_LIST_SELECTED", Sheet::Pledit, 227, 130, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_LOAD_LIST", Sheet::Pledit, 204, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_LOAD_LIST_SELECTED", Sheet::Pledit, 227, 149, 22, 18), // provisional: webamp, pending S01/S17
    def("PLAYLIST_ADD_MENU_BAR", Sheet::Pledit, 48, 111, 3, 54), // provisional: webamp, pending S01/S17
    def("PLAYLIST_REMOVE_MENU_BAR", Sheet::Pledit, 100, 111, 3, 72), // provisional: webamp, pending S01/S17
    def("PLAYLIST_SELECT_MENU_BAR", Sheet::Pledit, 150, 111, 3, 54), // provisional: webamp, pending S01/S17
    def("PLAYLIST_MISC_MENU_BAR", Sheet::Pledit, 200, 111, 3, 54), // provisional: webamp, pending S01/S17
    def("PLAYLIST_LIST_BAR", Sheet::Pledit, 250, 111, 3, 54), // provisional: webamp, pending S01/S17
    def("PLAYLIST_CLOSE_SELECTED", Sheet::Pledit, 52, 42, 9, 9), // provisional: webamp, pending S01/S17
    def("PLAYLIST_COLLAPSE_SELECTED", Sheet::Pledit, 62, 42, 9, 9), // provisional: webamp, pending S01/S17
    def("PLAYLIST_EXPAND_SELECTED", Sheet::Pledit, 150, 42, 9, 9), // provisional: webamp, pending S01/S17
    // --- eq_ex.bmp ---
    def("EQ_SHADE_BACKGROUND_SELECTED", Sheet::EqEx, 0, 0, 275, 14), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_BACKGROUND", Sheet::EqEx, 0, 15, 275, 14), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_VOLUME_SLIDER_LEFT", Sheet::EqEx, 1, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_VOLUME_SLIDER_CENTER", Sheet::EqEx, 4, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_VOLUME_SLIDER_RIGHT", Sheet::EqEx, 7, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_BALANCE_SLIDER_LEFT", Sheet::EqEx, 11, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_BALANCE_SLIDER_CENTER", Sheet::EqEx, 14, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_BALANCE_SLIDER_RIGHT", Sheet::EqEx, 17, 30, 3, 7), // provisional: webamp, pending S01/S17
    def("EQ_MAXIMIZE_BUTTON_ACTIVE", Sheet::EqEx, 1, 38, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_MINIMIZE_BUTTON_ACTIVE", Sheet::EqEx, 1, 47, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_CLOSE_BUTTON", Sheet::EqEx, 11, 38, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_SHADE_CLOSE_BUTTON_ACTIVE", Sheet::EqEx, 11, 47, 9, 9), // provisional: webamp, pending S01/S17
    // --- eqmain.bmp ---
    def("EQ_WINDOW_BACKGROUND", Sheet::EqMain, 0, 0, 275, 116), // provisional: webamp, pending S01/S17
    def("EQ_TITLE_BAR", Sheet::EqMain, 0, 149, 275, 14), // provisional: webamp, pending S01/S17
    def("EQ_TITLE_BAR_SELECTED", Sheet::EqMain, 0, 134, 275, 14), // provisional: webamp, pending S01/S17
    def("EQ_SLIDER_BACKGROUND", Sheet::EqMain, 13, 164, 209, 129), // provisional: webamp, pending S01/S17
    def("EQ_SLIDER_THUMB", Sheet::EqMain, 0, 164, 11, 11), // provisional: webamp, pending S01/S17
    def("EQ_SLIDER_THUMB_SELECTED", Sheet::EqMain, 0, 176, 11, 11), // provisional: webamp, pending S01/S17
    def("EQ_CLOSE_BUTTON", Sheet::EqMain, 0, 116, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_CLOSE_BUTTON_ACTIVE", Sheet::EqMain, 0, 125, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_MAXIMIZE_BUTTON_ACTIVE_FALLBACK", Sheet::EqMain, 254, 152, 9, 9), // provisional: webamp, pending S01/S17
    def("EQ_ON_BUTTON", Sheet::EqMain, 10, 119, 26, 12), // provisional: webamp, pending S01/S17
    def("EQ_ON_BUTTON_DEPRESSED", Sheet::EqMain, 128, 119, 26, 12), // provisional: webamp, pending S01/S17
    def("EQ_ON_BUTTON_SELECTED", Sheet::EqMain, 69, 119, 26, 12), // provisional: webamp, pending S01/S17
    def("EQ_ON_BUTTON_SELECTED_DEPRESSED", Sheet::EqMain, 187, 119, 26, 12), // provisional: webamp, pending S01/S17
    def("EQ_AUTO_BUTTON", Sheet::EqMain, 36, 119, 32, 12), // provisional: webamp, pending S01/S17
    def("EQ_AUTO_BUTTON_DEPRESSED", Sheet::EqMain, 154, 119, 32, 12), // provisional: webamp, pending S01/S17
    def("EQ_AUTO_BUTTON_SELECTED", Sheet::EqMain, 95, 119, 32, 12), // provisional: webamp, pending S01/S17
    def("EQ_AUTO_BUTTON_SELECTED_DEPRESSED", Sheet::EqMain, 213, 119, 32, 12), // provisional: webamp, pending S01/S17
    def("EQ_GRAPH_BACKGROUND", Sheet::EqMain, 0, 294, 113, 19), // provisional: webamp, pending S01/S17
    def("EQ_GRAPH_LINE_COLORS", Sheet::EqMain, 115, 294, 1, 19), // provisional: webamp, pending S01/S17
    def("EQ_PRESETS_BUTTON", Sheet::EqMain, 224, 164, 44, 12), // provisional: webamp, pending S01/S17
    def("EQ_PRESETS_BUTTON_SELECTED", Sheet::EqMain, 224, 176, 44, 12), // provisional: webamp, pending S01/S17
    def("EQ_PREAMP_LINE", Sheet::EqMain, 0, 314, 113, 1), // provisional: webamp, pending S01/S17
    // --- posbar.bmp ---
    def("MAIN_POSITION_SLIDER_BACKGROUND", Sheet::Posbar, 0, 0, 248, 10), // provisional: webamp, pending S01/S17
    def("MAIN_POSITION_SLIDER_THUMB", Sheet::Posbar, 248, 0, 29, 10), // provisional: webamp, pending S01/S17
    def("MAIN_POSITION_SLIDER_THUMB_SELECTED", Sheet::Posbar, 278, 0, 29, 10), // provisional: webamp, pending S01/S17
    // --- shufrep.bmp ---
    def("MAIN_SHUFFLE_BUTTON", Sheet::Shufrep, 28, 0, 47, 15), // provisional: webamp, pending S01/S17
    def("MAIN_SHUFFLE_BUTTON_DEPRESSED", Sheet::Shufrep, 28, 15, 47, 15), // provisional: webamp, pending S01/S17
    def("MAIN_SHUFFLE_BUTTON_SELECTED", Sheet::Shufrep, 28, 30, 47, 15), // provisional: webamp, pending S01/S17
    def("MAIN_SHUFFLE_BUTTON_SELECTED_DEPRESSED", Sheet::Shufrep, 28, 45, 47, 15), // provisional: webamp, pending S01/S17
    def("MAIN_REPEAT_BUTTON", Sheet::Shufrep, 0, 0, 28, 15), // provisional: webamp, pending S01/S17
    def("MAIN_REPEAT_BUTTON_DEPRESSED", Sheet::Shufrep, 0, 15, 28, 15), // provisional: webamp, pending S01/S17
    def("MAIN_REPEAT_BUTTON_SELECTED", Sheet::Shufrep, 0, 30, 28, 15), // provisional: webamp, pending S01/S17
    def("MAIN_REPEAT_BUTTON_SELECTED_DEPRESSED", Sheet::Shufrep, 0, 45, 28, 15), // provisional: webamp, pending S01/S17
    def("MAIN_EQ_BUTTON", Sheet::Shufrep, 0, 61, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_EQ_BUTTON_SELECTED", Sheet::Shufrep, 0, 73, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_EQ_BUTTON_DEPRESSED", Sheet::Shufrep, 46, 61, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_EQ_BUTTON_DEPRESSED_SELECTED", Sheet::Shufrep, 46, 73, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_PLAYLIST_BUTTON", Sheet::Shufrep, 23, 61, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_PLAYLIST_BUTTON_SELECTED", Sheet::Shufrep, 23, 73, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_PLAYLIST_BUTTON_DEPRESSED", Sheet::Shufrep, 69, 61, 23, 12), // provisional: webamp, pending S01/S17
    def("MAIN_PLAYLIST_BUTTON_DEPRESSED_SELECTED", Sheet::Shufrep, 69, 73, 23, 12), // provisional: webamp, pending S01/S17
    // --- titlebar.bmp ---
    def("MAIN_TITLE_BAR", Sheet::Titlebar, 27, 15, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_TITLE_BAR_SELECTED", Sheet::Titlebar, 27, 0, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_EASTER_EGG_TITLE_BAR", Sheet::Titlebar, 27, 72, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_EASTER_EGG_TITLE_BAR_SELECTED", Sheet::Titlebar, 27, 57, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_OPTIONS_BUTTON", Sheet::Titlebar, 0, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_OPTIONS_BUTTON_DEPRESSED", Sheet::Titlebar, 0, 9, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_MINIMIZE_BUTTON", Sheet::Titlebar, 9, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_MINIMIZE_BUTTON_DEPRESSED", Sheet::Titlebar, 9, 9, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BUTTON", Sheet::Titlebar, 0, 18, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BUTTON_DEPRESSED", Sheet::Titlebar, 9, 18, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_CLOSE_BUTTON", Sheet::Titlebar, 18, 0, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_CLOSE_BUTTON_DEPRESSED", Sheet::Titlebar, 18, 9, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BACKGROUND", Sheet::Titlebar, 304, 0, 8, 43), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BACKGROUND_DISABLED", Sheet::Titlebar, 312, 0, 8, 43), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BUTTON_O_SELECTED", Sheet::Titlebar, 304, 47, 8, 8), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BUTTON_A_SELECTED", Sheet::Titlebar, 312, 55, 8, 7), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BUTTON_I_SELECTED", Sheet::Titlebar, 320, 62, 8, 7), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BUTTON_D_SELECTED", Sheet::Titlebar, 328, 69, 8, 8), // provisional: webamp, pending S01/S17
    def("MAIN_CLUTTER_BAR_BUTTON_V_SELECTED", Sheet::Titlebar, 336, 77, 8, 7), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BACKGROUND", Sheet::Titlebar, 27, 42, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BACKGROUND_SELECTED", Sheet::Titlebar, 27, 29, 275, 14), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BUTTON_SELECTED", Sheet::Titlebar, 0, 27, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_BUTTON_SELECTED_DEPRESSED", Sheet::Titlebar, 9, 27, 9, 9), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_POSITION_BACKGROUND", Sheet::Titlebar, 0, 36, 17, 7), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_POSITION_THUMB", Sheet::Titlebar, 20, 36, 3, 7), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_POSITION_THUMB_LEFT", Sheet::Titlebar, 17, 36, 3, 7), // provisional: webamp, pending S01/S17
    def("MAIN_SHADE_POSITION_THUMB_RIGHT", Sheet::Titlebar, 23, 36, 3, 7), // provisional: webamp, pending S01/S17
    // --- volume.bmp ---
    def("MAIN_VOLUME_BACKGROUND", Sheet::Volume, 0, 0, 68, 420), // provisional: webamp, pending S01/S17
    def("MAIN_VOLUME_THUMB", Sheet::Volume, 15, 422, 14, 11), // provisional: webamp, pending S01/S17
    def("MAIN_VOLUME_THUMB_SELECTED", Sheet::Volume, 0, 422, 14, 11), // provisional: webamp, pending S01/S17
    // --- gen.bmp ---
    def("GEN_TOP_LEFT_SELECTED", Sheet::Gen, 0, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_LEFT_END_SELECTED", Sheet::Gen, 26, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_CENTER_FILL_SELECTED", Sheet::Gen, 52, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_RIGHT_END_SELECTED", Sheet::Gen, 78, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_LEFT_RIGHT_FILL_SELECTED", Sheet::Gen, 104, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_RIGHT_SELECTED", Sheet::Gen, 130, 0, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_LEFT", Sheet::Gen, 0, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_LEFT_END", Sheet::Gen, 26, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_CENTER_FILL", Sheet::Gen, 52, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_RIGHT_END", Sheet::Gen, 78, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_LEFT_RIGHT_FILL", Sheet::Gen, 104, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_TOP_RIGHT", Sheet::Gen, 130, 21, 25, 20), // provisional: webamp, pending S01/S17
    def("GEN_BOTTOM_LEFT", Sheet::Gen, 0, 42, 125, 14), // provisional: webamp, pending S01/S17
    def("GEN_BOTTOM_RIGHT", Sheet::Gen, 0, 57, 125, 14), // provisional: webamp, pending S01/S17
    def("GEN_BOTTOM_FILL", Sheet::Gen, 127, 72, 25, 14), // provisional: webamp, pending S01/S17
    def("GEN_MIDDLE_LEFT", Sheet::Gen, 127, 42, 11, 29), // provisional: webamp, pending S01/S17
    def("GEN_MIDDLE_LEFT_BOTTOM", Sheet::Gen, 158, 42, 11, 24), // provisional: webamp, pending S01/S17
    def("GEN_MIDDLE_RIGHT", Sheet::Gen, 139, 42, 8, 29), // provisional: webamp, pending S01/S17
    def("GEN_MIDDLE_RIGHT_BOTTOM", Sheet::Gen, 170, 42, 8, 24), // provisional: webamp, pending S01/S17
    def("GEN_CLOSE_SELECTED", Sheet::Gen, 148, 42, 9, 9), // provisional: webamp, pending S01/S17
];

/// The `text.bmp` cell size, ported from webamp (`CHAR_X` x `CHAR_Y`).
pub const TEXT_CELL: (u32, u32) = (5, 6); // provisional: webamp, pending S01/S17

/// The `text.bmp` glyph grid, ported from webamp's `FONT_LOOKUP`.
///
/// Each entry maps a character to its `(column, row)` cell. The grid is laid
/// out with 31 columns and a handful of rows; positions follow the classic
/// alphabet ordering including the Nordic `Å Ö Ä` and the punctuation block.
#[rustfmt::skip]
pub static TEXT_GLYPHS: &[(char, u32, u32)] = &[
    ('a', 0, 0), ('b', 0, 1), ('c', 0, 2), ('d', 0, 3), ('e', 0, 4), ('f', 0, 5), // provisional: webamp, pending S01/S17
    ('g', 0, 6), ('h', 0, 7), ('i', 0, 8), ('j', 0, 9), ('k', 0, 10), ('l', 0, 11), // provisional: webamp, pending S01/S17
    ('m', 0, 12), ('n', 0, 13), ('o', 0, 14), ('p', 0, 15), ('q', 0, 16), ('r', 0, 17), // provisional: webamp, pending S01/S17
    ('s', 0, 18), ('t', 0, 19), ('u', 0, 20), ('v', 0, 21), ('w', 0, 22), ('x', 0, 23), // provisional: webamp, pending S01/S17
    ('y', 0, 24), ('z', 0, 25), ('"', 0, 26), ('@', 0, 27), (' ', 0, 30), // provisional: webamp, pending S01/S17
    ('0', 1, 0), ('1', 1, 1), ('2', 1, 2), ('3', 1, 3), ('4', 1, 4), ('5', 1, 5), // provisional: webamp, pending S01/S17
    ('6', 1, 6), ('7', 1, 7), ('8', 1, 8), ('9', 1, 9), ('\u{2026}', 1, 10), // provisional: webamp, pending S01/S17
    ('.', 1, 11), (':', 1, 12), ('(', 1, 13), (')', 1, 14), ('-', 1, 15), // provisional: webamp, pending S01/S17
    ('\'', 1, 16), ('!', 1, 17), ('_', 1, 18), ('+', 1, 19), ('\\', 1, 20), // provisional: webamp, pending S01/S17
    ('/', 1, 21), ('[', 1, 22), (']', 1, 23), ('^', 1, 24), ('&', 1, 25), // provisional: webamp, pending S01/S17
    ('%', 1, 26), (',', 1, 27), ('=', 1, 28), ('$', 1, 29), ('#', 1, 30), // provisional: webamp, pending S01/S17
    ('Å', 2, 0), ('Ö', 2, 1), ('Ä', 2, 2), ('?', 2, 3), ('*', 2, 4), // provisional: webamp, pending S01/S17
    ('<', 1, 22), ('>', 1, 23), ('{', 1, 22), ('}', 1, 23), // provisional: webamp, pending S01/S17
];

/// The cursor files a skin may ship, ported from webamp's `CURSORS` list.
///
/// The trailing `v` marks the four cursor names webamp lists as unused
/// (`volbar.cur`, `wsclose.cur`, `wswinbut.cur`, `wsmin.cur`).
pub static CURSOR_NAMES: &[&str] = &[
    "close.cur",    // provisional: webamp, pending S01/S17
    "eqclose.cur",  // provisional: webamp, pending S01/S17
    "eqnormal.cur", // provisional: webamp, pending S01/S17
    "eqslid.cur",   // provisional: webamp, pending S01/S17
    "eqtitle.cur",  // provisional: webamp, pending S01/S17
    "mainmenu.cur", // provisional: webamp, pending S01/S17
    "mmenu.cur",    // provisional: webamp, pending S01/S17
    "min.cur",      // provisional: webamp, pending S01/S17
    "normal.cur",   // provisional: webamp, pending S01/S17
    "pclose.cur",   // provisional: webamp, pending S01/S17
    "pnormal.cur",  // provisional: webamp, pending S01/S17
    "posbar.cur",   // provisional: webamp, pending S01/S17
    "psize.cur",    // provisional: webamp, pending S01/S17
    "ptbar.cur",    // provisional: webamp, pending S01/S17
    "pvscroll.cur", // provisional: webamp, pending S01/S17
    "pwinbut.cur",  // provisional: webamp, pending S01/S17
    "pwsnorm.cur",  // provisional: webamp, pending S01/S17
    "pwssize.cur",  // provisional: webamp, pending S01/S17
    "songname.cur", // provisional: webamp, pending S01/S17
    "titlebar.cur", // provisional: webamp, pending S01/S17
    "volbal.cur",   // provisional: webamp, pending S01/S17
    "winbut.cur",   // provisional: webamp, pending S01/S17
    "wsnormal.cur", // provisional: webamp, pending S01/S17
    "wsposbar.cur", // provisional: webamp, pending S01/S17
];

/// The 24 default analyser colours, ported from webamp's `baseSkin.json`.
pub static DEFAULT_VISCOLORS: [[u8; 3]; 24] = [
    [0, 0, 0],       // provisional: webamp, pending S01/S17
    [24, 33, 41],    // provisional: webamp, pending S01/S17
    [239, 49, 16],   // provisional: webamp, pending S01/S17
    [206, 41, 16],   // provisional: webamp, pending S01/S17
    [214, 90, 0],    // provisional: webamp, pending S01/S17
    [214, 102, 0],   // provisional: webamp, pending S01/S17
    [214, 115, 0],   // provisional: webamp, pending S01/S17
    [198, 123, 8],   // provisional: webamp, pending S01/S17
    [222, 165, 24],  // provisional: webamp, pending S01/S17
    [214, 181, 33],  // provisional: webamp, pending S01/S17
    [189, 222, 41],  // provisional: webamp, pending S01/S17
    [148, 222, 33],  // provisional: webamp, pending S01/S17
    [41, 206, 16],   // provisional: webamp, pending S01/S17
    [50, 190, 16],   // provisional: webamp, pending S01/S17
    [57, 181, 16],   // provisional: webamp, pending S01/S17
    [49, 156, 8],    // provisional: webamp, pending S01/S17
    [41, 148, 0],    // provisional: webamp, pending S01/S17
    [24, 132, 8],    // provisional: webamp, pending S01/S17
    [255, 255, 255], // provisional: webamp, pending S01/S17
    [214, 214, 222], // provisional: webamp, pending S01/S17
    [181, 189, 189], // provisional: webamp, pending S01/S17
    [160, 170, 175], // provisional: webamp, pending S01/S17
    [148, 156, 165], // provisional: webamp, pending S01/S17
    [150, 150, 150], // provisional: webamp, pending S01/S17
];

/// Default playlist colour values, ported from webamp's `baseSkin.json`.
pub const DEFAULT_PLAYLIST_NORMAL: [u8; 3] = [0x00, 0xFF, 0x00]; // provisional: webamp, pending S01/S17
/// Default currently-playing colour.
pub const DEFAULT_PLAYLIST_CURRENT: [u8; 3] = [0xFF, 0xFF, 0xFF]; // provisional: webamp, pending S01/S17
/// Default playlist background.
pub const DEFAULT_PLAYLIST_NORMAL_BG: [u8; 3] = [0x00, 0x00, 0x00]; // provisional: webamp, pending S01/S17
/// Default selected-entry background.
pub const DEFAULT_PLAYLIST_SELECTED_BG: [u8; 3] = [0x00, 0x00, 0xFF]; // provisional: webamp, pending S01/S17
/// Default playlist font.
pub const DEFAULT_PLAYLIST_FONT: &str = "Arial"; // provisional: webamp, pending S01/S17

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
        assert_eq!((s.x, s.y, s.width, s.height), (23, 0, 23, 18));
        assert!(sprite("NOT_A_SPRITE").is_none());
    }

    #[test]
    fn nums_ex_falls_back_to_numbers() {
        assert_eq!(Sheet::NumsEx.fallback(), Some(Sheet::Numbers));
        assert_eq!(Sheet::Main.fallback(), None);
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
        assert_eq!(sheet_extent(Sheet::Numbers), (90, 13));
        assert_eq!(sheet_extent(Sheet::NumsEx), (108, 13));
    }
}
