// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-milkdrop
//!
//! The player's own MilkDrop visualizer. It needs no plug-in: the
//! [`lactea`](https://github.com/M0Rf30/lactea) engine (MIT OR Apache-2.0, with
//! Nullsoft's BSD-3-Clause MilkDrop 2 portions; see `THIRD_PARTY.md`) evaluates
//! the presets and renders them with `wgpu`. This crate is what a player has to
//! add to that engine:
//!
//! * [`presets`]: the preset folder and the presets the player brings along;
//! * [`selection`]: next, previous and random, over the engine's playlist;
//! * [`audio`]: the player's decoded audio, as the PCM the engine analyses;
//! * [`session`]: the worker thread that owns the GPU and hands out finished
//!   frames as pixels, so that a window can draw them like everything else.
//!
//! The frames come back as pixels rather than through a swap chain on purpose:
//! the window carries a skinned title bar drawn on the CPU, as the media library
//! does, and the same path works on a machine without a GPU, where Direct3D 12's
//! software adapter renders.

pub mod audio;
mod gpu;
pub mod presets;
mod render;
pub mod selection;
pub mod session;
mod textures;

pub use session::{RenderedFrame, Request, Session, Settings, Status};
