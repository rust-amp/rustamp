// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The OS window of the player's own MilkDrop ([`crate::milkdrop_window`]): the
//! same borderless, resizable, skinned window as the media library, painted
//! through `softbuffer`. It is painted when the worker has a new frame.

use std::num::NonZeroU32;
use std::sync::Arc;

use rustamp_milkdrop::Status;
use rustamp_ui::shell::TaskbarRole;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Fullscreen, Window};

use super::{Windowed, map_key, map_modifiers, resize_edge};
use crate::milkdrop_window::{KeyAction, MILKDROP_WINDOW_SIZE, key_action};

/// The presenter's half of the window.
#[derive(Default)]
pub(super) struct MilkdropPresenter {
    pub(super) window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    cursor: (i32, i32),
    /// What was last painted; a tick that has nothing new paints nothing.
    painted: Option<(u64, Option<Status>)>,
    /// The fullscreen state last applied to the OS window.
    fullscreen: bool,
}

impl Windowed {
    /// Creates or drops the OS window to match the app's model, and paints it
    /// when there is something new.
    pub(super) fn sync_milkdrop_window(&mut self, event_loop: &ActiveEventLoop) {
        if !self.app.milkdrop_window().is_open() {
            self.milkdrop = MilkdropPresenter::default();
            return;
        }
        self.open_milkdrop_window(event_loop);
        let wanted = self.app.milkdrop_window().fullscreen();
        if wanted != self.milkdrop.fullscreen {
            self.milkdrop.fullscreen = wanted;
            if let Some(window) = &self.milkdrop.window {
                window.set_fullscreen(wanted.then_some(Fullscreen::Borderless(None)));
                // The move of the client area arrives as a resize.
                self.app
                    .milkdrop_window()
                    .resized(window.inner_size().into());
            }
            self.milkdrop.painted = None;
        }
        if self.milkdrop.painted.as_ref() != Some(&self.app.milkdrop_window().content_key()) {
            self.paint_milkdrop();
        }
    }

    fn open_milkdrop_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.milkdrop.window.is_some() {
            return;
        }
        let attributes = crate::modern_shell::modern_window_attributes(
            "MilkDrop",
            MILKDROP_WINDOW_SIZE,
            TaskbarRole::Secondary,
        )
        .with_position(PhysicalPosition::new(40, 64));
        let opened = event_loop
            .create_window(attributes)
            .map_err(|error| error.to_string())
            .and_then(|window| {
                let window = Arc::new(window);
                let context = softbuffer::Context::new(Arc::clone(&window))
                    .map_err(|error| error.to_string())?;
                let surface = softbuffer::Surface::new(&context, Arc::clone(&window))
                    .map_err(|error| error.to_string())?;
                Ok((window, surface))
            });
        match opened {
            Ok((window, surface)) => {
                self.milkdrop.window = Some(window);
                self.milkdrop.surface = Some(surface);
                self.milkdrop.painted = None;
            }
            Err(error) => {
                eprintln!("rustamp: could not create the MilkDrop window: {error}");
                self.app.close_milkdrop_window();
            }
        }
    }

    fn paint_milkdrop(&mut self) {
        let Some(window) = self.milkdrop.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        self.milkdrop.painted = Some(self.app.milkdrop_window().content_key());
        let image = self
            .app
            .render_milkdrop_window((width, height), window.has_focus());
        let Some(surface) = self.milkdrop.surface.as_mut() else {
            return;
        };
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        for (pixel, rgba) in buffer.iter_mut().zip(image.as_rgba().as_chunks::<4>().0) {
            *pixel = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
        }
        let _ = buffer.present();
    }

    /// Events of the window, in client coordinates. Closing it stops the session
    /// and never quits the player.
    pub(super) fn milkdrop_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.app.close_milkdrop_window();
                self.sync_milkdrop_window(event_loop);
            }
            WindowEvent::RedrawRequested => self.paint_milkdrop(),
            WindowEvent::Resized(size) => {
                self.app.milkdrop_window().resized(size.into());
                self.paint_milkdrop();
            }
            WindowEvent::Focused(_) => self.milkdrop.painted = None,
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.milkdrop.window {
                    self.app
                        .milkdrop_window()
                        .resized(window.inner_size().into());
                }
                self.milkdrop.painted = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.milkdrop.cursor = (position.x as i32, position.y as i32);
                if let Some(window) = &self.milkdrop.window
                    && !self.milkdrop.fullscreen
                {
                    let edge = resize_edge(window.inner_size(), self.milkdrop.cursor);
                    window.set_cursor(edge.map_or(winit::window::CursorIcon::Default, |edge| {
                        winit::window::CursorIcon::from(edge)
                    }));
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.milkdrop_click(event_loop),
            // A key still held when the window takes the focus was meant for
            // the window it was pressed in.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
            WindowEvent::KeyboardInput { event, .. } => self.milkdrop_key(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            _ => {}
        }
    }

    fn milkdrop_click(&mut self, event_loop: &ActiveEventLoop) {
        if self.milkdrop.fullscreen {
            return;
        }
        let Some(window) = self.milkdrop.window.clone() else {
            return;
        };
        let (x, y) = self.milkdrop.cursor;
        let size = window.inner_size();
        if rustamp_ui::chrome::hits_close(size.width, x, y) {
            self.app.close_milkdrop_window();
            self.sync_milkdrop_window(event_loop);
        } else if let Some(edge) = resize_edge(size, (x, y)) {
            let _ = window.drag_resize_window(edge);
        } else if rustamp_ui::chrome::hits_title_bar(y) {
            let _ = window.drag_window();
        }
    }

    fn milkdrop_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        let fullscreen = self.milkdrop.fullscreen;
        match key_action(key, modifiers, fullscreen) {
            Some(KeyAction::Preset(request)) => self.app.milkdrop_window().request(request),
            Some(KeyAction::ToggleFullscreen) => {
                self.app.milkdrop_window_mut().set_fullscreen(!fullscreen);
            }
            Some(KeyAction::LeaveFullscreen) => {
                self.app.milkdrop_window_mut().set_fullscreen(false);
            }
            None => {
                if let Err(error) = self.app.key_down(key, modifiers) {
                    eprintln!("rustamp: shortcut dispatch failed: {error}");
                }
            }
        }
    }
}
