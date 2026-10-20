// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-ui
//!
//! Window components and widgets, hit testing, docking and snapping,
//! windowshade, double-size, easy-move and menus.
//!
//! The classic **main window** is implemented (task cards D1/D2, milestone M1):
//!
//! * [`layout::MainWindowLayout`] holds every rectangle spec **S01** measured,
//!   in one place, each annotated with its S01 section. Values S01 marks
//!   *provisional* stay marked provisional.
//! * [`state::MainWindowState`] is a plain data struct describing what a single
//!   frame shows (transport state, clutter-bar flags, sliders, time, title,
//!   kbps/kHz, mono/stereo, pressed buttons).
//! * [`behaviour::MainWindow`] is the headless **behaviour** state machine of
//!   spec **S02**: transport, clutterbar, marquee, time display, hints and mouse
//!   dispatch, driven by [`rustamp_core::command::Command`] and
//!   [`rustamp_core::event::Event`]. Time is a parameter, never a clock.
//! * [`shortcuts`] maps key presses to the S03 main-window actions and
//!   dispatches them on [`behaviour::MainWindow`] (task card D3).
//! * [`menu`] holds the main-menu popup model (wave21b): the measured rows in
//!   one table, the check-state mapping and the popup renderer. Row order and
//!   grouping stay provisional until the S04 spec exports.
//! * [`render::render_main_window`] paints one RGBA frame through
//!   [`rustamp_gfx`], falling back per spec **S17** when a skin lacks a sheet.
//! * [`widget::hit_test`] maps a window-relative point to a [`widget::Widget`].
//! * [`cursor`] maps a hit-test result to the [`rustamp_skin::CursorSlot`] the
//!   skin is asked for. The mapping is **Rustamp's own choice** (S17 §8
//!   leaves it open) and is a pure function: it never touches the skin or
//!   the OS.
//! * [`visualization`] implements Rustamp's own analyser/scope and envelopes,
//!   consuming actual waveform snapshots. S08 reference behaviour remains open.
//! * [`playlist::PlaylistWindow`] is the headless playlist-editor behaviour of
//!   spec **S06**: list selection, page scrolling, REM removal and the
//!   bottom-bar dialog requests (titles only, no dialog internals).
//! * [`library::LibraryView`] is the headless media-library view model: the
//!   loaded page of rows, the search text, the sort key and direction, the
//!   selected row and the scroll offset. It never touches SQLite - the app
//!   runs the query and hands the rows back.
//! * [`library_layout::LibraryLayout`] is the renderer-facing layout of the
//!   library window: the columns, their widths in text cells and the row
//!   lines as text. The library window is a modern (unskinned) window, so no
//!   skin is involved. The **cover column** in front of the text columns is
//!   a fixed number of pixels wide
//!   ([`library_layout::LIBRARY_COVER_WIDTH`]): it holds the album art, and
//!   a picture has no font metrics.
//! * [`library_render::render_library_window`] paints the media library's
//!   content - the search field, the column titles, the rows, the
//!   selection, the scrollbar and the status line - into a frame, with the
//!   modern window's own palette. The renderer takes the covers of the
//!   visible rows as borrowed views
//!   ([`library_render::CoverRef`]) and draws them into the cover column's
//!   cell of their row. [`library_render::LibraryChrome`] is the
//!   window's geometry: a pure function of the viewport and the view, so
//!   the app can map a click to the band it belongs to.
//! * [`placement`] is the clamp-to-screen rule: a window whose saved
//!   position points at a screen that is gone is placed so that its title
//!   bar - the only part of a borderless window the user can drag - stays
//!   reachable on a screen.
//! * [`shell::skinned_window_attributes`] is the single place that decides
//!   what an OS window looks like outside its client area: every window of
//!   this player is borderless, because its title bar is part of the skin.
//! * [`presenter::MainWindowPresenter`] is the thin `winit`/`softbuffer`
//!   window used by the `main_window` example; it owns the clock.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod behaviour;
pub mod cursor;
pub mod dialogs;
pub mod docking;
pub mod eq_render;
pub mod equalizer;
// Compatibility entry points for the same module under other common names.
pub use docking as geometry;
pub use docking as windows;
pub mod layout;
// The seeded property suite for the layout math. It lives beside the code
// it tests because it needs the screen types of `rustamp-platform`, which is
// a dependency of this crate; a test in `tests/` would need them as a
// dev-dependency, and the crate takes no new dependency.
#[cfg(test)]
mod layout_props;
pub mod library;
pub mod library_layout;
pub mod library_render;
pub mod menu;
pub mod placement;
pub mod playlist;
mod playlist_font;
pub mod playlist_window;
pub mod presenter;
pub mod render;
pub mod shell;
pub mod shortcuts;
pub mod state;
pub mod visualization;
pub mod widget;

pub use behaviour::{
    MainWindow, PROGRAM_NAME, balance_percent, core_balance, core_volume, transport_transition,
    volume_percent,
};
pub use cursor::{equalizer_cursor, main_window_cursor, playlist_cursor_at};
pub use dialogs::{
    ABOUT_CLIENT_SIZE, DISC_TOTAL_KEY, DialogKind, DialogManager, EDIT_ENTRY_CLIENT_SIZE,
    EDIT_ENTRY_HINT, EDIT_ENTRY_TITLE, EXTENDED_INFO_CLIENT_SIZE, EditEntryChange,
    EditEntryChanges, EditEntryDialog, EditEntryError, EditEntryField, EditEntryInvalid,
    EditEntryNumber, EditEntryOutcome, ExtendedInfo, FILE_INFO_CLIENT_SIZE, JUMP_CLIENT_SIZE,
    JUMP_FILE_CLIENT_SIZE, JUMP_FILE_ROWS, LOAD_PLAYLIST_CLIENT_SIZE, MAX_EDIT_ENTRY_GENRE_LEN,
    MAX_EDIT_ENTRY_NUMBER, MAX_EDIT_ENTRY_NUMBER_LEN, MAX_EDIT_ENTRY_TEXT_LEN, MAX_EDIT_ENTRY_YEAR,
    MAX_EDIT_ENTRY_YEAR_LEN, MAX_JUMP_FILE_LEN, MAX_JUMP_LEN, MAX_MEDIA_LEN, MIN_EDIT_ENTRY_YEAR,
    OPEN_MEDIA_CLIENT_SIZE, PREFS_CLIENT_SIZE, SKIN_BROWSER_CLIENT_SIZE, SkinEntry, TagField,
    dialog_for_command, edit_entry_lines, extended_info_lines, media_item_for_url,
    media_items_for_paths, opening_command, parse_jump_time_ms, playlist_load_summary_lines,
    render_dialog, skin_browser_visible_rows, skin_entries,
};
pub use docking::{
    ClassicWindow, DEFAULT_EQUALIZER_POSITION, DEFAULT_MAIN_POSITION, DEFAULT_PLAYLIST_POSITION,
    DOUBLE_EQUALIZER_SIZE, DOUBLE_MAIN_SIZE, DockedStack, MoveTestPositions,
    PLAYLIST_UNCHANGED_SIZE, Point, Position, SHADE_REFLOW_PX, SHADED_MAIN_SIZE, SNAP_TOLERANCE_PX,
    WindowKind, WindowPosition, WindowSize, bottom_edge, equalizer_top, move_test_positions,
    neighbour_tracks, playlist_top, snap_candidate, snap_edge, snap_within, window_size,
};
pub use eq_render::{eq_frame_size, render_equalizer_window};
pub use equalizer::{
    BALANCE_CENTRE_LEFT, BALANCE_LEFT_EXTREME, BALANCE_RIGHT_EXTREME, BALANCE_THUMB_WIDTH,
    BALANCE_TRACK_LEFT, BALANCE_TRACK_WIDTH, BAND_COUNT, BAND_PITCH, BAND_X0,
    EQUALIZER_WINDOW_SIZE, EqMenuRequest, EqSlider, EqWidget, EqualizerLayout, EqualizerState,
    EqualizerWindow, FLAT_VALUE, MAX_VALUE, PREAMP_X, PRESETS_MENU_ENTRIES,
    PRESETS_POPUP_CLIENT_SIZE, PRESETS_POPUP_WINDOW_SIZE, THUMB_BOTTOM_Y, THUMB_FLAT_Y, THUMB_SIZE,
    THUMB_TOP_Y, VOLUME_THUMB_WIDTH, VOLUME_TRAVEL_LEFT, VOLUME_TRAVEL_RIGHT,
    balance_hint_for_thumb, hit_test as eq_hit_test, saved_to_gain_db, thumb_y_for_value,
    value_for_thumb_y, volume_core_from_click, volume_display_percent,
};
pub use layout::{
    DOUBLE_HEIGHT, DOUBLE_WIDTH, MainWindowLayout, SHADE_HEIGHT, WINDOW_HEIGHT, WINDOW_WIDTH,
};
pub use library::{LibraryColumn, LibraryRequest, LibraryRow, LibrarySort, LibraryView};
pub use library_layout::{
    LIBRARY_COLUMN_GAP, LIBRARY_COLUMN_TITLES, LIBRARY_COLUMN_WIDTHS, LIBRARY_COLUMNS,
    LIBRARY_COVER_WIDTH, LIBRARY_MIN_TITLE_WIDTH, LibraryLayout, format_duration,
};
pub use library_render::{
    CoverRef, LIBRARY_CARET_WIDTH, LIBRARY_CELL_WIDTH, LIBRARY_FONT_FAMILY, LIBRARY_HEADER_TEXT,
    LIBRARY_LINE_HEIGHT, LIBRARY_MIN_THUMB_HEIGHT, LIBRARY_PANEL_BACKGROUND, LIBRARY_ROW_TEXT,
    LIBRARY_SCROLLBAR_THUMB, LIBRARY_SCROLLBAR_TRACK, LIBRARY_SCROLLBAR_WIDTH,
    LIBRARY_SEARCH_BACKGROUND, LIBRARY_SEARCH_CARET, LIBRARY_SEARCH_HINT, LIBRARY_SEARCH_PADDING,
    LIBRARY_SEARCH_PLACEHOLDER, LIBRARY_SEARCH_TEXT, LIBRARY_SELECTION, LIBRARY_SELECTION_TEXT,
    LIBRARY_SEPARATOR, LIBRARY_STATUS_TEXT, LIBRARY_WINDOW_BACKGROUND, LibraryChrome, cover_cell,
    render_library_window, scroll_for_click, text_column_x,
};
pub use menu::{
    CHECK_CELL_X, CLUTTER_O_POINT, CLUTTER_V_POINT, MAIN_MENU, MAIN_MENU_CLASS,
    MAIN_MENU_CLIENT_SIZE, MAIN_MENU_ITEM_HEIGHT_PX, MAIN_MENU_ROWS, MAIN_MENU_SEPARATOR_HEIGHT_PX,
    MAIN_MENU_SEPARATORS, MAIN_MENU_SUBMENUS, MENU_BUTTON_POINT, MainMenuId, MainMenuItem,
    MenuAction, MenuCheck, MenuChoice, MenuFlags, MenuRow, NativeMainMenu, SubmenuId, SubmenuItem,
    check_state, clutter_popup_origin, menu_item_at, menu_popup_origin, native_main_menu,
    native_options_menu, native_visualization_menu, render_main_menu, render_submenu, row_at_y,
    row_bands, submenu_client_size, submenu_entries, top_level,
};
pub use placement::{
    clamp_into_work_area, clamp_stack_to_screen, clamp_to_screen, title_bar_height,
    title_bar_inside, title_bar_rect,
};
pub use playlist::{
    ADD_DIALOG_TITLE, BUTTON_BAND_BOTTOM, BUTTON_BAND_TOP, LIST_SELECT_POINT, LOAD_DIALOG_TITLE,
    PLAYLIST_TABLE, PLAYLIST_WINDOW_SIZE, PlaylistBinding, PlaylistButton, PlaylistDialog,
    PlaylistKeyAction, PlaylistWindow, ScrollDirection, UNIMPLEMENTED_PLAYLIST_ROWS, VISIBLE_ROWS,
    button_hit_range, has_window_menu_button, hit_test_button, playlist_key_action,
    playlist_key_binding, unimplemented_playlist_row,
};
pub use playlist_font::{
    FONT_SIZE_STEP, MAX_FONT_SIZE, MIN_FONT_SIZE, PlaylistFontSize, skin_font_size,
};
pub use playlist_window::{
    BOTTOM_BAR_RECT, BUTTON_SRC_Y, CLOSE_BUTTON_RECT, LIST_BOTTOM, LIST_LEFT, LIST_OPTS_SRC_X,
    LIST_PAGE_HEIGHT, LIST_RIGHT, LIST_TOP, PlaylistTitleButton, ROW_PITCH, SCROLL_ARROWS_RECT,
    SCROLL_ARROWS_SRC, SCROLL_HANDLE_SRC, SCROLL_HANDLE_X, SCROLL_SPLIT_Y, SCROLL_TOP_Y,
    SCROLL_TRACK_RECT, SCROLL_TRAVEL_PX, SHADE_BUTTON_RECT, ScrollbarHit, TITLE_BAR_HEIGHT,
    TITLE_STRIP_WIDTH, TITLE_STRIP_X, TITLE_TILE_WIDTH, button_rect, hit_test_list_row,
    hit_test_scroll, hit_test_scrollbar, hit_test_title_bar, in_list_area, list_area,
    render_playlist_window, row_pitch, row_rect, scroll_handle_rect, scroll_handle_y,
    scroll_offset_for_handle_y, visible_rows,
};
pub use presenter::{MainWindowPresenter, PresenterError, map_key, map_key_code, map_modifiers};
pub use render::{frame_size, render_main_window};
pub use shell::{
    SKINNED_DECORATIONS, SKINNED_RESIZABLE, TaskbarRole, attributes_client_size,
    skinned_window_attributes,
};
pub use shortcuts::{
    Binding, GLOBAL_TABLE, Key, MAIN_TABLE, Modifiers, ShortcutAction, ShortcutOutcome,
    ShortcutWindow, TEN_TRACK_STEP, VOLUME_STEP, lookup,
};
pub use state::{MainWindowState, MonoStereo, TransportButton, TransportState, WindowButton};
pub use widget::{Widget, hit_test};
