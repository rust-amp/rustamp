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
//! Keyboard events are mapped here (and only here) from `winit` types into the
//! headless [`Key`] / [`Modifiers`] of [`crate::shortcuts`], dispatched with
//! [`MainWindow::key_down`], and the outcome is printed to stdout. A held key
//! repeats: each OS key-repeat event dispatches the action again.
//!
//! # Window sizing and DPI (task card F1)
//!
//! The layout is fixed in physical pixels (275x116, 550x232 double-sized; see
//! `specs/S01-window-geometry.md`). [`MainWindowPresenter::run`] makes the
//! process per-monitor-V2 DPI aware (via `rustamp-platform`) before the
//! `winit` event loop exists, then sizes the OS window by its **client area**:
//! every size request goes through `request_inner_size` with the exact layout
//! pixels, and `winit` translates that to an outer size with the real window
//! chrome (`AdjustWindowRect` on Windows). The pure chrome math
//! (`rustamp_platform::outer_size_for_client`) takes numbers, not a window,
//! and is unit-tested in `rustamp-platform`; `winit` does not expose per-side
//! chrome insets before the window is shown, so there is nothing more precise
//! to request up front.
//!
//! Toggling double-size (or windowshade) resizes the OS window through the
//! same path and the softbuffer surface follows on the next paint. On a DPI
//! change while running (`ScaleFactorChanged`) the presenter re-asserts the
//! exact layout size in physical pixels via the size writer, and on any resize
//! that would leave the client area smaller than the layout it re-requests
//! the layout size; the window therefore never settles below the layout. It
//! keeps its pixel size across monitors (smaller-looking on high DPI);
//! logical-size preservation belongs to the app layer or S05.
//!
//! Tests never call [`MainWindowPresenter::run`]; only the `main_window`
//! example does, on an explicit manual run.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use rustamp_gfx::Image;
use rustamp_platform::init_dpi_awareness;
use rustamp_skin::Skin;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::behaviour::MainWindow;
use crate::layout::MainWindowLayout;
use crate::render::{frame_size, render_main_window};
use crate::shortcuts::{Key, Modifiers, ShortcutOutcome};
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
    modifiers: ModifiersState,
    window_handle: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// The client size last requested from the OS window, so repeat requests
    /// for an unchanged size are skipped.
    applied_client: Option<(u32, u32)>,
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
            modifiers: ModifiersState::empty(),
            window_handle: None,
            surface: None,
            applied_client: None,
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

    /// The client area the OS window must have right now, in physical pixels
    /// (the S01 layout size, doubled when double-sized).
    fn desired_client_size(&self) -> (u32, u32) {
        frame_size(self.window.state())
    }

    /// Resizes the OS window so its client area is exactly
    /// [`Self::desired_client_size`], unless that size was already requested.
    ///
    /// The request names the client area, so window chrome needs no manual
    /// inset here; the softbuffer surface follows on the next paint. This is
    /// what makes toggling double-size (or windowshade) resize the OS window
    /// instead of only the painted surface.
    fn sync_window_size(&mut self) {
        let desired = self.desired_client_size();
        if self.applied_client == Some(desired) {
            return;
        }
        if let Some(window) = &self.window_handle {
            let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
            self.applied_client = Some(desired);
        }
    }

    /// Opens the window and runs the event loop until it is closed.
    ///
    /// This blocks and must run on the main thread. It never opens a window
    /// during `cargo test` because no test calls it.
    ///
    /// The process is made per-monitor-V2 DPI aware before the event loop is
    /// created, so the client area comes up in physical pixels. If that fails
    /// the window still opens (`winit` retries awareness itself) with a
    /// warning on stderr; the size is then whatever the OS virtualizes.
    pub fn run(mut self) -> Result<(), PresenterError> {
        if let Err(error) = init_dpi_awareness() {
            eprintln!(
                "rustamp-ui: DPI awareness unavailable ({error}); the client area may be virtualized"
            );
        }
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

    /// Dispatches one keyboard event through the shortcut table.
    fn key_event(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        let now = self.elapsed_ms();
        match self.window.key_down(key, modifiers, now) {
            Some(ShortcutOutcome::Commands(commands)) => {
                println!("rustamp-ui: shortcut {key:?} -> {commands:?}");
            }
            Some(ShortcutOutcome::Handled) => {
                println!("rustamp-ui: shortcut {key:?} handled locally");
            }
            Some(ShortcutOutcome::Unhandled(action)) => {
                println!(
                    "rustamp-ui: shortcut {key:?} -> {action:?} (command id {}, no Command yet)",
                    action.command_id()
                );
            }
            None => return,
        }
        // A shortcut may have toggled double-size or windowshade, which
        // changes the client size the OS window must have.
        self.sync_window_size();
        self.request_redraw();
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
        // The creation attributes already name the client area; re-assert it
        // once the window exists in case the OS virtualized the request.
        self.sync_window_size();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Resized(size) => {
                let (width, height) = self.desired_client_size();
                if size.width < width || size.height < height {
                    // Never settle for less than the layout (the DPI-change
                    // floor); forget what was applied so the request repeats.
                    self.applied_client = None;
                    self.sync_window_size();
                }
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                // A monitor/DPI change suggests a scaled size; decline it and
                // keep the exact layout size in physical pixels.
                let (width, height) = self.desired_client_size();
                let _ = inner_size_writer.request_inner_size(PhysicalSize::new(width, height));
                self.applied_client = Some((width, height));
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
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
                // A click may have toggled double-size (the clutter-bar `D`),
                // which changes the client size the OS window must have.
                self.sync_window_size();
                self.request_redraw();
            }
            _ => {}
        }
    }
}

/// Converts `winit` modifier state into the headless [`Modifiers`].
fn map_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers::new(state.control_key(), state.alt_key(), state.shift_key())
}

/// Maps a physical key code to a [`Key`] (keys whose position, not their
/// character, matters: keypad, navigation and function keys, digits, grave).
fn map_key_code(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Numpad0 => Key::Numpad(0),
        KeyCode::Numpad1 => Key::Numpad(1),
        KeyCode::Numpad2 => Key::Numpad(2),
        KeyCode::Numpad3 => Key::Numpad(3),
        KeyCode::Numpad4 => Key::Numpad(4),
        KeyCode::Numpad5 => Key::Numpad(5),
        KeyCode::Numpad6 => Key::Numpad(6),
        KeyCode::Numpad7 => Key::Numpad(7),
        KeyCode::Numpad8 => Key::Numpad(8),
        KeyCode::Numpad9 => Key::Numpad(9),
        KeyCode::NumpadDecimal => Key::NumDecimal,
        KeyCode::NumpadAdd => Key::NumAdd,
        KeyCode::NumpadSubtract => Key::NumSubtract,
        KeyCode::Digit0 => Key::Digit(0),
        KeyCode::Digit1 => Key::Digit(1),
        KeyCode::Digit2 => Key::Digit(2),
        KeyCode::Digit3 => Key::Digit(3),
        KeyCode::Digit4 => Key::Digit(4),
        KeyCode::Digit5 => Key::Digit(5),
        KeyCode::Digit6 => Key::Digit(6),
        KeyCode::Digit7 => Key::Digit(7),
        KeyCode::Digit8 => Key::Digit(8),
        KeyCode::Digit9 => Key::Digit(9),
        KeyCode::F1 => Key::F(1),
        KeyCode::F2 => Key::F(2),
        KeyCode::F3 => Key::F(3),
        KeyCode::F4 => Key::F(4),
        KeyCode::F5 => Key::F(5),
        KeyCode::F6 => Key::F(6),
        KeyCode::F7 => Key::F(7),
        KeyCode::F8 => Key::F(8),
        KeyCode::F9 => Key::F(9),
        KeyCode::F10 => Key::F(10),
        KeyCode::F11 => Key::F(11),
        KeyCode::F12 => Key::F(12),
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::Insert => Key::Insert,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backquote => Key::Grave,
        KeyCode::Space => Key::Space,
        KeyCode::Escape => Key::Escape,
        _ => return None,
    })
}

/// Maps a `winit` key event to a [`Key`]: letters by their (layout-aware)
/// character, everything else by physical key code.
fn map_key(event: &KeyEvent) -> Option<Key> {
    if let WinitKey::Character(text) = &event.logical_key {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(letter) = Key::letter(c)
            && !matches!(event.physical_key, PhysicalKey::Code(code) if map_key_code(code).is_some_and(|k| matches!(k, Key::Numpad(_))))
        {
            return Some(letter);
        }
    }
    match event.physical_key {
        PhysicalKey::Code(code) => map_key_code(code),
        PhysicalKey::Unidentified(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_codes_map_to_headless_keys() {
        assert_eq!(map_key_code(KeyCode::Numpad7), Some(Key::Numpad(7)));
        assert_eq!(map_key_code(KeyCode::NumpadDecimal), Some(Key::NumDecimal));
        assert_eq!(map_key_code(KeyCode::ArrowLeft), Some(Key::Left));
        assert_eq!(map_key_code(KeyCode::F5), Some(Key::F(5)));
        assert_eq!(map_key_code(KeyCode::Digit3), Some(Key::Digit(3)));
        assert_eq!(map_key_code(KeyCode::Backquote), Some(Key::Grave));
        assert_eq!(map_key_code(KeyCode::KeyA), None);
    }

    #[test]
    fn modifier_state_maps_to_the_headless_set() {
        let state = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(map_modifiers(state), Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(map_modifiers(ModifiersState::empty()), Modifiers::NONE);
        assert_eq!(map_modifiers(ModifiersState::ALT), Modifiers::ALT);
    }
}
