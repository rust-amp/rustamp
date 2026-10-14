//! A thin `winit` + `softbuffer` presenter for the classic main window.
//!
//! The presenter owns a [`Skin`] and a headless [`MainWindow`] behaviour, reads
//! the wall clock, injects it into the machine ([`MainWindow::refresh`]) and
//! blits one frame to a `softbuffer` surface. The clock lives **here**, never in
//! the behaviour module, so the machine stays deterministic.
//!
//! It forwards the left mouse button to the machine's [`MainWindow::mouse_down`]
//! / [`MainWindow::mouse_move`] / [`MainWindow::mouse_up`] so the example is
//! interactive; the commands the machine emits have no engine to reach yet, so
//! they are dropped (the machine applies its own S02 prediction).
//!
//! Tests never call [`MainWindowPresenter::run`]; only the `main_window`
//! example does, on an explicit manual run.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use rustamp_gfx::Image;
use rustamp_skin::Skin;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::behaviour::MainWindow;
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

/// Owns the behaviour state and paints frames.
pub struct MainWindowPresenter {
    skin: Skin,
    window: MainWindow,
    layout: MainWindowLayout,
    started: Option<Instant>,
    cursor: (i32, i32),
    left_down: bool,
    window_handle: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
}

impl MainWindowPresenter {
    /// Creates a presenter for `skin` and the behaviour `window`.
    pub fn new(skin: Skin, window: MainWindow) -> Self {
        Self {
            skin,
            window,
            layout: MainWindowLayout::CLASSIC,
            started: None,
            cursor: (0, 0),
            left_down: false,
            window_handle: None,
            surface: None,
        }
    }

    /// Creates a presenter that starts from a single render state.
    pub fn from_state(skin: Skin, state: MainWindowState) -> Self {
        Self::new(skin, MainWindow::from_state(state))
    }

    /// The behaviour machine, for feeding events and commands.
    pub fn window(&self) -> &MainWindow {
        &self.window
    }

    /// The behaviour machine, mutably.
    pub fn window_mut(&mut self) -> &mut MainWindow {
        &mut self.window
    }

    /// The state that will be painted on the next redraw.
    pub fn state(&self) -> &MainWindowState {
        self.window.state()
    }

    /// Renders the current state to an RGBA frame, refreshing the time-driven
    /// parts with the elapsed clock first.
    pub fn render(&mut self) -> Image {
        self.window.refresh(self.elapsed_ms());
        render_main_window(self.window.state(), &self.skin, &self.layout)
    }

    /// Asks the platform for a redraw.
    pub fn request_redraw(&self) {
        if let Some(window) = &self.window_handle {
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

    fn elapsed_ms(&self) -> u64 {
        self.started
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    /// Converts a physical cursor position to the layout's 1x coordinates.
    fn to_layout(&self, position: PhysicalPosition<f64>) -> (i32, i32) {
        let scale = if self.window.state().double_size {
            2.0
        } else {
            1.0
        };
        ((position.x / scale) as i32, (position.y / scale) as i32)
    }

    fn paint(&mut self) {
        let image = self.render();
        let (width, height) = frame_size(self.window.state());
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
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
        if self.window_handle.is_some() {
            return;
        }
        let (width, height) = frame_size(self.window.state());
        let attributes = Window::default_attributes()
            .with_title("Rustamp - main window (D2)")
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
        self.started = Some(Instant::now());
        self.window_handle = Some(window);
        self.surface = Some(surface);
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.to_layout(position);
                if self.left_down {
                    let now = self.elapsed_ms();
                    let (x, y) = self.cursor;
                    let _ = self.window.mouse_move(x, y, now);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let now = self.elapsed_ms();
                let (x, y) = self.cursor;
                match state {
                    ElementState::Pressed => {
                        self.left_down = true;
                        let _ = self.window.mouse_down(x, y, now);
                    }
                    ElementState::Released => {
                        self.left_down = false;
                        let _ = self.window.mouse_up(x, y, now);
                    }
                }
                self.request_redraw();
            }
            _ => {}
        }
    }
}
