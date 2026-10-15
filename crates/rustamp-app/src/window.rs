// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The windowed presenter: a `winit` + `softbuffer` shell around [`App`].
//!
//! The app owns the OS and the clock (task card N1): this module creates the
//! window, forwards OS input into [`App`], pumps the app on a ~60 Hz timer so
//! the engine's events reach the UI machine and the marquee/time animate, and
//! blits the frame [`App::render`] produces.
//!
//! # Window sizing (task card F1's contract)
//!
//! The layout is fixed in physical pixels (275x116, 550x232 double-sized;
//! `specs/S01-window-geometry.md`). The process is made per-monitor-V2 DPI
//! aware before the event loop, and every size request names the **client
//! area** through `request_inner_size`, so toggling double-size (the clutter
//! bar's `D`) or windowshade resizes the OS window, not just the painted
//! surface.
//!
//! # Why this is not `rustamp_ui::MainWindowPresenter`
//!
//! That presenter is a manual example: it owns its own [`MainWindow`] and
//! **drops** the commands the machine emits, so it cannot route anything to the
//! engine. It is also private about its `winit` key mapping. The app therefore
//! drives `winit` itself and duplicates the (short) physical-key mapping; see
//! the findings in the task card report.
//!
//! # GUI rule: the window may only run inside the Windows Sandbox
//!
//! Opening this window is a GUI run. Per the project rule, GUI must never run
//! on the host: the reference Winamp is driven inside a Windows Sandbox on this
//! machine, and a second window on the host would interfere with those
//! captures. The default `cargo test --workspace --all-features` therefore
//! **never** reaches [`run`]: the app's tests use `--null` (see
//! [`crate::app::App::run_headless`]), and this function refuses to open a
//! window when [`NO_GUI_ENV`] is set, which the test harness always does. The
//! one test that does open a window is `#[ignore]`d, sandbox-only, and refuses
//! unless it runs as the sandbox user
//! (`crates/rustamp-app/tests/headless.rs`). See `crates/rustamp-app/README.md`.
//!
//! **A spawned child must not create a window either — a console window counts
//! as a window on the host.** Test and tool code that spawns a process passes
//! `CREATE_NO_WINDOW`; `tests/headless.rs` centralises that in `no_window`.
//!
//! No test calls [`run`] outside that ignored, sandbox-gated case.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustamp_platform::init_dpi_awareness;
use rustamp_ui::{Key, Modifiers, ShortcutOutcome};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::app::App;

/// The repaint/pump interval (~60 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Environment variable that forbids opening the GUI window.
///
/// The test harness (`crates/rustamp-app/tests/headless.rs`) always sets it, so
/// a test that accidentally loses `--null` fails loudly instead of opening a
/// window. It is a safety rail for automated runs, not a product switch: a
/// human running the player normally does not set it. GUI runs belong inside
/// the Windows Sandbox.
pub const NO_GUI_ENV: &str = "RUSTAMP_NO_GUI";

/// Opt-in that lets a debug build open a window anyway.
///
/// The rail below is **fail-closed**: a debug run refuses unless it runs as the
/// sandbox account or sets this. Release builds are unaffected, so the shipped
/// player behaves normally.
pub const ALLOW_GUI_ENV: &str = "RUSTAMP_ALLOW_GUI";

/// The Windows Sandbox account. Everything inside the sandbox runs as this user,
/// which is the marker that a run is not on the host.
pub const SANDBOX_USER: &str = "WDAGUtilityAccount";

/// Fails when [`NO_GUI_ENV`] is set.
///
/// The app calls this before it builds anything, so a mis-run fails
/// immediately and never reaches the audio device or the event loop.
/// Fails unless this process may open a window.
///
/// **Fail-closed in debug builds.** A development or test run may only open a
/// window when it provably runs inside the Windows Sandbox (the sandbox account
/// is [`SANDBOX_USER`]) or when it opted in with [`ALLOW_GUI_ENV`]. On the
/// owner's desktop a debug run therefore **cannot** put a window up, whatever
/// started it — a test, a stray `cargo run`, an agent. The shipped (release)
/// player is unaffected and opens its window normally.
///
/// The owner's rule is absolute: nothing graphical runs on the host
/// (`docs/HANDOFF.md`, the GUI rule).
pub fn ensure_gui_allowed() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os(NO_GUI_ENV).is_some() {
        return Err(format!(
            "the windowed player is disabled by {NO_GUI_ENV} (GUI runs only inside the \
             Windows Sandbox)"
        )
        .into());
    }
    #[cfg(debug_assertions)]
    {
        let in_sandbox =
            std::env::var("USERNAME").is_ok_and(|user| user.eq_ignore_ascii_case(SANDBOX_USER));
        if !in_sandbox && std::env::var_os(ALLOW_GUI_ENV).is_none() {
            return Err(format!(
                "refusing to open a window: this is a debug build on the host, and GUI runs \
                 only inside the Windows Sandbox. Run it there (the sandbox account is \
                 {SANDBOX_USER}), or set {ALLOW_GUI_ENV}=1 if you really mean it."
            )
            .into());
        }
    }
    Ok(())
}

/// Opens the main window and runs until it is closed.
///
/// Blocks and must run on the main thread. Never called by a test, except the
/// `#[ignore]`d, sandbox-only one. Returns an error without opening anything
/// when [`NO_GUI_ENV`] is set.
pub fn run(app: App) -> Result<(), Box<dyn std::error::Error>> {
    ensure_gui_allowed()?;
    if let Err(error) = init_dpi_awareness() {
        eprintln!(
            "rustamp: DPI awareness unavailable ({error}); the client area may be virtualized"
        );
    }
    let mut presenter = Windowed::new(app);
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut presenter)?;
    Ok(())
}

struct Windowed {
    app: App,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    cursor: (i32, i32),
    left_down: bool,
    modifiers: ModifiersState,
    /// The client size last requested, so repeat requests are skipped.
    applied_client: Option<(u32, u32)>,
    /// The always-on-top level last applied to the OS window.
    applied_on_top: bool,
}

impl Windowed {
    fn new(app: App) -> Self {
        let applied_on_top = app.always_on_top();
        Self {
            app,
            window: None,
            surface: None,
            cursor: (0, 0),
            left_down: false,
            modifiers: ModifiersState::empty(),
            applied_client: None,
            applied_on_top,
        }
    }

    /// Resizes the OS window so its client area is exactly the layout size, and
    /// applies the always-on-top and minimize requests.
    fn sync_window_state(&mut self) {
        let desired = self.app.frame_size();
        if self.applied_client != Some(desired)
            && let Some(window) = &self.window
        {
            let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
            self.applied_client = Some(desired);
        }
        if let Some(window) = &self.window {
            if self.app.always_on_top() != self.applied_on_top {
                self.applied_on_top = self.app.always_on_top();
                window.set_window_level(if self.applied_on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                });
            }
            if self.app.take_minimize_request() {
                window.set_minimized(true);
            }
        }
    }

    /// Converts a physical cursor position to the layout's 1x coordinates.
    fn to_layout(&self, position: PhysicalPosition<f64>) -> (i32, i32) {
        let scale = if self.app.state().double_size {
            2.0
        } else {
            1.0
        };
        ((position.x / scale) as i32, (position.y / scale) as i32)
    }

    fn key_event(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        match self.app.key_down(key, modifiers) {
            Ok(Some(ShortcutOutcome::Commands(commands))) => {
                println!("rustamp: shortcut {key:?} -> {commands:?}");
            }
            Ok(Some(ShortcutOutcome::Handled)) => {
                println!("rustamp: shortcut {key:?} handled locally");
            }
            Ok(Some(ShortcutOutcome::Unhandled(action))) => {
                println!(
                    "rustamp: shortcut {key:?} -> {action:?} (command id {}, no Command yet)",
                    action.command_id()
                );
            }
            Ok(None) => return,
            Err(error) => eprintln!("rustamp: shortcut dispatch failed: {error}"),
        }
        self.sync_window_state();
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn paint(&mut self) {
        let image = self.app.render();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
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

impl ApplicationHandler for Windowed {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (width, height) = self.app.frame_size();
        let attributes = Window::default_attributes()
            .with_title("Rustamp")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the window: {error}");
                event_loop.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp: could not create the softbuffer context: {error}");
                event_loop.exit();
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp: could not create the softbuffer surface: {error}");
                event_loop.exit();
                return;
            }
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = surface.resize(width, height);
        }
        self.window = Some(window);
        self.surface = Some(surface);
        self.sync_window_state();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.app.request_quit();
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Resized(size) => {
                let (width, height) = self.app.frame_size();
                if size.width < width || size.height < height {
                    // Never settle below the layout (the DPI-change floor).
                    self.applied_client = None;
                    self.sync_window_state();
                }
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                let (width, height) = self.app.frame_size();
                let _ = inner_size_writer.request_inner_size(PhysicalSize::new(width, height));
                self.applied_client = Some((width, height));
                self.request_redraw();
            }
            WindowEvent::Focused(focused) => {
                self.app.set_focused(focused);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.to_layout(position);
                if self.left_down {
                    let (x, y) = self.cursor;
                    if let Err(error) = self.app.mouse_move(x, y) {
                        eprintln!("rustamp: pointer move failed: {error}");
                    }
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.cursor;
                let result = match state {
                    ElementState::Pressed => {
                        self.left_down = true;
                        self.app.mouse_down(x, y)
                    }
                    ElementState::Released => {
                        self.left_down = false;
                        self.app.mouse_up(x, y)
                    }
                };
                if let Err(error) = result {
                    eprintln!("rustamp: pointer input failed: {error}");
                }
                // A click may have toggled double-size (the clutter-bar `D`).
                self.sync_window_state();
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.app.pump() {
            eprintln!("rustamp: pump failed: {error}");
        }
        for note in self.app.drain_notes() {
            eprintln!("rustamp: {note}");
        }
        if self.app.quit() {
            event_loop.exit();
            return;
        }
        self.sync_window_state();
        self.request_redraw();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }
}

/// Converts `winit` modifier state into the headless [`Modifiers`].
fn map_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers::new(state.control_key(), state.alt_key(), state.shift_key())
}

/// Maps a physical key code to a [`Key`].
///
/// This duplicates `rustamp_ui::presenter`'s private mapping (see the module
/// docs); it is the project's own code, kept in sync by hand.
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
