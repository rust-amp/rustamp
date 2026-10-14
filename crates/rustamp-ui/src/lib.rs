//! # rustamp-ui
//!
//! Window components and widgets, hit testing, docking and snapping,
//! windowshade, double-size, easy-move and menus.
//!
//! The first implemented piece is the classic **main window** (task card D1,
//! milestone M1):
//!
//! * [`layout::MainWindowLayout`] holds every rectangle spec **S01** measured,
//!   in one place, each annotated with its S01 section. Values S01 marks
//!   *provisional* stay marked provisional.
//! * [`state::MainWindowState`] is a plain data struct describing what a single
//!   frame shows (transport state, clutter-bar flags, sliders, time, title,
//!   kbps/kHz, mono/stereo). There are **no** transitions, timers or shortcuts
//!   here: those belong to spec S02 and later cards.
//! * [`render::render_main_window`] paints one RGBA frame through
//!   [`rustamp_gfx`], falling back per spec **S17** when a skin lacks a sheet.
//! * [`widget::hit_test`] maps a window-relative point to a [`widget::Widget`]
//!   so a later card can wire clicks without re-deriving the geometry.
//! * [`presenter::MainWindowPresenter`] is the thin `winit`/`softbuffer`
//!   window used by the `main_window` example.
//!
//! Codes against the frozen contracts in [`rustamp_core`]; see
//! `docs/rfc/0001-core-contracts.md`.

#![forbid(unsafe_code)]

pub mod layout;
pub mod presenter;
pub mod render;
pub mod state;
pub mod widget;

pub use layout::{
    DOUBLE_HEIGHT, DOUBLE_WIDTH, MainWindowLayout, SHADE_HEIGHT, WINDOW_HEIGHT, WINDOW_WIDTH,
};
pub use presenter::{MainWindowPresenter, PresenterError};
pub use render::{frame_size, render_main_window};
pub use state::{MainWindowState, MonoStereo, TransportButton, TransportState};
pub use widget::{Widget, hit_test};
