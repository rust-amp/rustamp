//! A thin `winit` + `softbuffer` presenter for the classic main window.
//!
//! The presenter is deliberately small: it owns a [`Skin`] and a
//! [`MainWindowState`], renders one frame with [`render_main_window`] and blits
//! it to a `softbuffer` surface. It handles no input beyond closing the window,
//! so the app lane can drive it later (change the state, call
//! [`MainWindowPresenter::set_state`] and request a redraw).
//!
//! Tests never call [`MainWindowPresenter::run`]; only the `main_window`
//! example does, on an explicit manual run.

use std::num::NonZeroU32;
use std::sync::Arc;

use rustamp_gfx::Image;
use rustamp_skin::Skin;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::layout::MainWindowLayout;
use crate::render::{frame_size, render_main_window};
use crate::state::MainWindowState;

/// A failure while opening or driving the window.
#[derive(Debug, thiserror::Error)]
pub enum PresenterError {
    /// The event loop could not be created or run.
    #[error("winit event loop: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    /// The window could not be created.
    #[error("winit window: {0}")]
    Window(#[from] winit::error::OsError),
    /// The softbuffer surface could not be created or presented.
    #[error("softbuffer: {0}")]
    Surface(#[from] softbuffer::SoftBufferError),
}

/// Owns the window state and paints frames.
pub struct MainWindowPresenter {
    skin: Skin,
    state: MainWindowState,
    layout: MainWindowLayout,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
}

impl MainWindowPresenter {
    /// Creates a presenter for `skin`, starting from `state`.
    pub fn new(skin: Skin, state: MainWindowState) -> Self {
        Self {
            skin,
            state,
            layout: MainWindowLayout::CLASSIC,
            window: None,
            surface: None,
        }
    }

    /// The state that will be painted on the next redraw.
    pub fn state(&self) -> &MainWindowState {
        &self.state
    }

    /// Replaces the state. The window is not repainted until the platform
    /// requests a redraw (or [`Self::request_redraw`] is called).
    pub fn set_state(&mut self, state: MainWindowState) {
        self.state = state;
    }

    /// Renders the current state to an RGBA frame.
    pub fn render(&self) -> Image {
        render_main_window(&self.state, &self.skin, &self.layout)
    }

    /// Asks the platform for a redraw.
    pub fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Opens the window and runs the event loop until it is closed.
    ///
    /// This blocks and must run on the main thread. It never opens a window
    /// during `cargo test` because no test calls it.
    pub fn run(mut self) -> Result<(), PresenterError> {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut self)?;
        Ok(())
    }

    fn paint(&mut self) {
        let (width, height) = frame_size(&self.state);
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        // Render before borrowing the surface, so `self` is not borrowed twice.
        let image = self.render();
        let Some(surface) = self.surface.as_mut() else {
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
}

impl ApplicationHandler for MainWindowPresenter {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (width, height) = frame_size(&self.state);
        let attributes = Window::default_attributes()
            .with_title("Rustamp - main window (D1)")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp-ui: could not create the window: {error}");
                event_loop.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp-ui: could not create the softbuffer context: {error}");
                event_loop.exit();
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp-ui: could not create the softbuffer surface: {error}");
                event_loop.exit();
                return;
            }
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = surface.resize(width, height);
        }
        self.window = Some(window);
        self.surface = Some(surface);
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.paint(),
            _ => {}
        }
    }
}
