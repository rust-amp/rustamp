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
//! * [`render::render_main_window`] paints one RGBA frame through
//!   [`rustamp_gfx`], falling back per spec **S17** when a skin lacks a sheet.
//! * [`widget::hit_test`] maps a window-relative point to a [`widget::Widget`].
//! * [`playlist::PlaylistWindow`] is the headless playlist-editor behaviour of
//!   spec **S06**: list selection, page scrolling, REM removal and the
//!   bottom-bar dialog requests (titles only, no dialog internals).
//! * [`presenter::MainWindowPresenter`] is the thin `winit`/`softbuffer`
//!   window used by the `main_window` example; it owns the clock.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod behaviour;
pub mod docking;
pub mod eq_render;
pub mod equalizer;
// Compatibility entry points for the same module under other common names.
pub use docking as geometry;
pub use docking as windows;
pub mod layout;
pub mod playlist;
pub mod presenter;
pub mod render;
pub mod shortcuts;
pub mod state;
pub mod widget;

pub use behaviour::{
    MainWindow, PROGRAM_NAME, balance_percent, core_balance, core_volume, transport_transition,
    volume_percent,
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
pub use playlist::{
    ADD_DIALOG_TITLE, BUTTON_BAND_BOTTOM, BUTTON_BAND_TOP, LIST_SELECT_POINT, LOAD_DIALOG_TITLE,
    PAGE_STEP_ROWS, PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistDialog, PlaylistWindow,
    ScrollDirection, VISIBLE_ROWS, button_hit_range, has_window_menu_button, hit_test_button,
};
pub use presenter::{MainWindowPresenter, PresenterError, map_key, map_key_code, map_modifiers};
pub use render::{frame_size, render_main_window};
pub use shortcuts::{
    Binding, GLOBAL_TABLE, Key, MAIN_TABLE, Modifiers, ShortcutAction, ShortcutOutcome,
    ShortcutWindow, TEN_TRACK_STEP, VOLUME_STEP, lookup,
};
pub use state::{MainWindowState, MonoStereo, TransportButton, TransportState, WindowButton};
pub use widget::{Widget, hit_test};
