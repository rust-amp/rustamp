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
//! * [`presenter::MainWindowPresenter`] is the thin `winit`/`softbuffer`
//!   window used by the `main_window` example; it owns the clock.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod behaviour;
pub mod layout;
pub mod presenter;
pub mod render;
pub mod shortcuts;
pub mod state;
pub mod widget;

pub use behaviour::{
    MainWindow, PROGRAM_NAME, balance_percent, core_balance, core_volume, transport_transition,
    volume_percent,
};
pub use layout::{
    DOUBLE_HEIGHT, DOUBLE_WIDTH, MainWindowLayout, SHADE_HEIGHT, WINDOW_HEIGHT, WINDOW_WIDTH,
};
pub use presenter::{MainWindowPresenter, PresenterError};
pub use render::{frame_size, render_main_window};
pub use shortcuts::{
    Binding, GLOBAL_TABLE, Key, MAIN_TABLE, Modifiers, ShortcutAction, ShortcutOutcome,
    ShortcutWindow, VOLUME_STEP, lookup,
};
pub use state::{MainWindowState, MonoStereo, TransportButton, TransportState, WindowButton};
pub use widget::{Widget, hit_test};
