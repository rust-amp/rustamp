// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The windowed presenter: `winit` + `softbuffer` shells around [`App`].
//!
//! The app owns the OS and the clock (task card N1): this module creates the
//! windows, forwards OS input into [`App`], pumps the app on a ~60 Hz timer so
//! the engine's events reach the UI machine and the marquee/time animate, and
//! blits the frames [`App::render`] and [`App::render_eq`] produce.
//!
//! Two OS windows exist when the equalizer is open (specs **S07** + **S05**):
//! the main window and the equalizer window (`Winamp EQ`, 275 x 116 at rest,
//! 550 x 232 double-sized; `specs/S01-window-geometry.md`, `specs/S05-docking.md`).
//! The equalizer is a **docked neighbour**: it is placed below the main
//! window's outer frame and re-placed whenever the main moves or resizes, so it
//! follows the main and tracks its bottom edge (S05 §4.2, §3.3). The client
//! geometry (sizes, the 102 px shade reflow, the same-delta move) is the
//! existing [`rustamp_ui::docking::DockedStack`], not new arithmetic; the one
//! approximation is documented on [`Windowed::place_eq_below_main`].
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
//! # GUI rule: the window may only run inside the headless test VM
//!
//! Opening this window is a GUI run. Per the project rule, GUI must never run
//! on the host: everything graphical runs in the disposable Hyper-V test VM
//! (`rustamp-test`, pinned to 100 % scale), and a window on the host would
//! break that rule. The default `cargo test --workspace --all-features`
//! therefore **never** reaches [`run`]: the app's tests use `--null` (see
//! [`crate::app::App::run_headless`]), and this function refuses to open a
//! window when [`NO_GUI_ENV`] is set, which the test harness always does. The
//! one test that does open a window is `#[ignore]`d, VM-only, and refuses
//! unless it runs inside the test VM
//! (`crates/rustamp-app/tests/headless.rs`). See `crates/rustamp-app/README.md`.
//!
//! **A spawned child must not create a window either — a console window counts
//! as a window on the host.** Test and tool code that spawns a process passes
//! `CREATE_NO_WINDOW`; `tests/headless.rs` centralises that in `no_window`.
//!
//! No test calls [`run`] outside that ignored, VM-gated case.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustamp_core::command::Command;
use rustamp_core::state::WindowId as CoreWindowId;
use rustamp_platform::init_dpi_awareness;
use rustamp_ui::{
    DEFAULT_MAIN_POSITION, DialogKind, Key, Modifiers, PLAYLIST_WINDOW_SIZE, ShortcutOutcome,
    docking::DockedStack,
    menu::{MAIN_MENU_CLIENT_SIZE, menu_popup_origin},
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use crate::app::App;

/// The repaint/pump interval (~60 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Environment variable that forbids opening the GUI window.
///
/// The test harness (`crates/rustamp-app/tests/headless.rs`) always sets it, so
/// a test that accidentally loses `--null` fails loudly instead of opening a
/// window. It is a safety rail for automated runs, not a product switch: a
/// human running the player normally does not set it. GUI runs belong inside
/// the headless test VM.
pub const NO_GUI_ENV: &str = "RUSTAMP_NO_GUI";

/// Opt-in that lets a debug build open a window anyway.
///
/// The rail below is **fail-closed**: a debug run refuses unless it runs
/// inside the test VM or sets this. Release builds are unaffected, so the
/// shipped player behaves normally.
pub const ALLOW_GUI_ENV: &str = "RUSTAMP_ALLOW_GUI";

/// Computer name of the disposable headless test VM, as last reported by the
/// test runner.
///
/// This is the fallback signal in [`is_test_vm`]: the primary signal is the
/// runner's staging tree ([`TEST_VM_MARKER`]), which survives a VM rebuild or
/// rename, while this name pins the VM as observed today. Either one proves
/// the run is not on the host.
pub const TEST_VM_COMPUTER: &str = "DESKTOP-UL3J9H6";

/// Path the test runner stages the repository tree to inside the guest
/// (`docs/notes/vm-run-tests.ps1` copies the staged tree to
/// `C:\rustamp\code\Rustamp` and maps it to `D:\code\Rustamp` via `subst`).
///
/// The host never has this path (its repository lives at `D:\code\Rustamp`
/// and its staging area elsewhere), so its presence proves the process runs
/// inside the disposable headless test VM. This is the primary signal in
/// [`is_test_vm`]: unlike a computer name it survives a VM rebuild.
pub const TEST_VM_MARKER: &str = r"C:\rustamp\code\Rustamp";

/// True when this process provably runs inside the disposable headless test
/// VM, the only host for anything graphical.
///
/// Either signal counts: the runner's staged tree ([`TEST_VM_MARKER`]) or the
/// VM's computer name ([`TEST_VM_COMPUTER`]). Neither is a wildcard — a plain
/// developer machine has neither — and there is deliberately **no**
/// environment-variable override: this is the predicate the window-opening
/// tests gate on, and an overridable test gate is a loaded gun. (The
/// [`ensure_gui_allowed`] rail keeps its own explicit [`ALLOW_GUI_ENV`]
/// opt-in for deliberate manual runs.)
pub fn is_test_vm() -> bool {
    if std::path::Path::new(TEST_VM_MARKER).exists() {
        return true;
    }
    std::env::var("COMPUTERNAME").is_ok_and(|name| name.eq_ignore_ascii_case(TEST_VM_COMPUTER))
}

/// Refuses unless this process provably runs inside the disposable headless
/// test VM.
///
/// The single gate all window-opening tests share (instead of a copy in
/// each): call this before anything graphical exists. There is deliberately
/// **no** override. GUI must never run on the host.
pub fn require_test_vm() {
    assert!(
        is_test_vm(),
        "refusing to open a GUI window: this test is VM-only, but this machine is neither the \
         headless test VM ({TEST_VM_COMPUTER}) nor carries the runner's staged tree \
         ({TEST_VM_MARKER}). GUI must never run on the host; run it inside the headless test \
         VM (see crates/rustamp-app/README.md)."
    );
}

/// Fails when [`NO_GUI_ENV`] is set.
///
/// The app calls this before it builds anything, so a mis-run fails
/// immediately and never reaches the audio device or the event loop.
/// Fails unless this process may open a window.
///
/// **Fail-closed in debug builds.** A development or test run may only open a
/// window when it provably runs inside the headless test VM (see
/// [`is_test_vm`]) or when it opted in with [`ALLOW_GUI_ENV`]. On the
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
             headless test VM)"
        )
        .into());
    }
    #[cfg(debug_assertions)]
    {
        if !is_test_vm() && std::env::var_os(ALLOW_GUI_ENV).is_none() {
            return Err(format!(
                "refusing to open a window: this is a debug build on the host, and GUI runs \
                 only inside the headless test VM. Run it there, or set {ALLOW_GUI_ENV}=1 if \
                 you really mean it."
            )
            .into());
        }
    }
    Ok(())
}

/// Opens the main window (plus the equalizer window when it is open and the
/// playlist window) and runs until the main window is closed.
///
/// Blocks and must run on the main thread. Never called by a test, except the
/// `#[ignore]`d, VM-only one. Returns an error without opening anything
/// when [`NO_GUI_ENV`] is set. Both windows share this rail: the gate runs
/// before either window exists.
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
    main_id: Option<WindowId>,
    /// The equalizer's OS window (S07; `None` while the EQ window is closed).
    eq_window: Option<Arc<Window>>,
    eq_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    eq_id: Option<WindowId>,
    /// The playlist editor window (`Winamp PE`, 275 x 116 at rest; S06, S01
    /// §4): a docked neighbour that follows the main window's bottom edge
    /// through the S05 geometry (see [`Windowed::sync_playlist_position`]).
    /// Created by the same shell and rail as the main window.
    playlist_window: Option<Arc<Window>>,
    playlist_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// The simple modal dialog window (About, file info, jump-to,
    /// load-playlist, open-media, skin browser, preferences; see `dialogs.rs`): at most one is open,
    /// created by the same shell and rail as the main window. Sizes and
    /// contents are own choices, except the load-playlist title (no spec
    /// covers dialog geometry; S04 is not started).
    dialog_window: Option<Arc<Window>>,
    dialog_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    dialog_id: Option<WindowId>,
    /// Which dialog the OS window above shows (`None` while it is closed).
    dialog_kind: Option<DialogKind>,
    /// The main-menu popup window (S04): at most one is open, created by the
    /// same shell and rail as the main window. The client size is the measured
    /// 323 x 569 ([`MAIN_MENU_CLIENT_SIZE`], S04 §§1,8); the reference class
    /// `#32768` (S04 §1) is not reproduced — `winit` cannot set a window
    /// class — so this is an ordinary small window hosting the rendered menu
    /// model. Placement, dismissal and keyboard handling are own choices:
    /// S04 §9 leaves popup placement and the OS popup's behaviour unmeasured.
    menu_window: Option<Arc<Window>>,
    menu_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    menu_id: Option<WindowId>,
    cursor: (i32, i32),
    eq_cursor: (i32, i32),
    menu_cursor: (i32, i32),
    left_down: bool,
    eq_left_down: bool,
    playlist_cursor: (i32, i32),
    modifiers: ModifiersState,
    /// The client size last requested, so repeat requests are skipped.
    applied_client: Option<(u32, u32)>,
    /// The equalizer's client size last requested.
    applied_eq_client: Option<(u32, u32)>,
    /// The always-on-top level last applied to the OS window.
    applied_on_top: bool,
    /// The docked stack model (S05 §3.3, §4.2, §5): shade/double-size flags
    /// and same-delta moves go through this instead of new arithmetic. Its
    /// absolute positions are the S05 default model; OS placement reads the
    /// live outer frame (see `place_eq_below_main`).
    stack: DockedStack,
    /// The main window's last seen outer position, for same-delta tracking.
    last_main_outer: Option<PhysicalPosition<i32>>,
}

impl Windowed {
    fn new(app: App) -> Self {
        let applied_on_top = app.always_on_top();
        Self {
            app,
            window: None,
            surface: None,
            main_id: None,
            eq_window: None,
            eq_surface: None,
            eq_id: None,
            playlist_window: None,
            playlist_surface: None,
            dialog_window: None,
            dialog_surface: None,
            dialog_id: None,
            dialog_kind: None,
            menu_window: None,
            menu_surface: None,
            menu_id: None,
            cursor: (0, 0),
            eq_cursor: (0, 0),
            menu_cursor: (0, 0),
            left_down: false,
            eq_left_down: false,
            playlist_cursor: (0, 0),
            modifiers: ModifiersState::empty(),
            applied_client: None,
            applied_eq_client: None,
            applied_on_top,
            stack: DockedStack::default_launch(),
            last_main_outer: None,
        }
    }

    /// Resizes the OS window so its client area is exactly the layout size, and
    /// applies the always-on-top and minimize requests. Also keeps the docked
    /// stack model in step with the main window's shade/double-size flags
    /// (S05 §3.3, §5: the 102 px reflow lives in the stack, not here).
    fn sync_window_state(&mut self) {
        let desired = self.app.frame_size();
        if self.applied_client != Some(desired)
            && let Some(window) = &self.window
        {
            let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
            self.applied_client = Some(desired);
        }
        let shaded = self.app.state().windowshade;
        if shaded != self.stack.main.shaded {
            self.stack.set_main_shaded(shaded);
        }
        let double_size = self.app.state().double_size;
        if double_size != self.stack.main.double_size {
            self.stack.set_main_double_size(double_size);
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

    /// Creates, resizes or closes the equalizer's OS window so it matches
    /// `eq_open` and [`App::eq_frame_size`].
    ///
    /// Creation needs the event loop, so this runs wherever the loop is
    /// available (`resumed`, `about_to_wait`, the EQ close path). A failed EQ
    /// creation is reported and skipped: the main window is unaffected.
    fn sync_eq_window(&mut self, event_loop: &ActiveEventLoop) {
        if !self.app.eq_open() {
            if self.eq_window.is_some() {
                self.eq_window = None;
                self.eq_surface = None;
                self.eq_id = None;
                self.applied_eq_client = None;
            }
            return;
        }
        if self.eq_window.is_none() {
            let (width, height) = self.app.eq_frame_size();
            let attributes = Window::default_attributes()
                .with_title("Rustamp Equalizer")
                .with_inner_size(PhysicalSize::new(width, height))
                .with_resizable(false);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    eprintln!("rustamp: could not create the equalizer window: {error}");
                    return;
                }
            };
            self.eq_id = Some(window.id());
            let context = match softbuffer::Context::new(Arc::clone(&window)) {
                Ok(context) => context,
                Err(error) => {
                    eprintln!(
                        "rustamp: could not create the equalizer softbuffer context: {error}"
                    );
                    self.eq_id = None;
                    return;
                }
            };
            let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
                Ok(surface) => surface,
                Err(error) => {
                    eprintln!(
                        "rustamp: could not create the equalizer softbuffer surface: {error}"
                    );
                    self.eq_id = None;
                    return;
                }
            };
            if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                let _ = surface.resize(width, height);
            }
            self.eq_window = Some(window);
            self.eq_surface = Some(surface);
            self.applied_eq_client = Some((width, height));
            self.place_eq_below_main();
        }
        let desired = self.app.eq_frame_size();
        if self.applied_eq_client != Some(desired)
            && let Some(window) = &self.eq_window
        {
            let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
            self.applied_eq_client = Some(desired);
        }
    }

    /// Places the equalizer window directly below the main window's outer
    /// frame, at the same x (the docked neighbour of S05 §3.3/§4.2).
    ///
    /// Approximation, stated plainly: the reference stacks *client* edges
    /// gap-free, but `winit` does not expose per-side chrome insets before or
    /// after the window is shown (the same reason the presenter never adjusts
    /// for chrome), so the client-edge-exact offset cannot be computed. The
    /// outer-frame-to-outer-frame placement keeps the frames touching and
    /// still tracks the main window's bottom edge through every move and
    /// resize; a shaded (shorter) main pulls the equalizer up with it.
    fn place_eq_below_main(&mut self) {
        let (Some(main), Some(eq)) = (self.window.as_ref(), self.eq_window.as_ref()) else {
            return;
        };
        if let Ok(pos) = main.outer_position() {
            let size = main.outer_size();
            eq.set_outer_position(PhysicalPosition::new(
                pos.x,
                pos.y.saturating_add(size.height as i32),
            ));
        }
    }

    /// Converts a physical cursor position to the equalizer layout's 1x
    /// coordinates (S05 §2.1: the flag reaches the equalizer, so its scale
    /// mirrors the main window's).
    fn to_eq_layout(&self, position: PhysicalPosition<f64>) -> (i32, i32) {
        let scale = if self.app.eq_state().double_size {
            2.0
        } else {
            1.0
        };
        ((position.x / scale) as i32, (position.y / scale) as i32)
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

    /// A key press while the equalizer window is focused (S03 §3 via S07 §1).
    fn eq_key_event(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        match self.app.eq_key_down(key, modifiers) {
            Ok(commands) if commands.is_empty() => {}
            Ok(commands) => println!("rustamp: equalizer shortcut {key:?} -> {commands:?}"),
            Err(error) => eprintln!("rustamp: equalizer shortcut dispatch failed: {error}"),
        }
        self.sync_window_state();
        self.request_redraw_eq();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn request_redraw_eq(&self) {
        if let Some(window) = &self.eq_window {
            window.request_redraw();
        }
    }

    fn request_playlist_redraw(&self) {
        if let Some(window) = &self.playlist_window {
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

    /// Blits the equalizer frame ([`App::render_eq`]) to its surface.
    fn paint_eq(&mut self) {
        let image = self.app.render_eq();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
            return;
        };
        let Some(surface) = self.eq_surface.as_mut() else {
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

    fn paint_playlist(&mut self) {
        let image = self.app.render_playlist();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
            return;
        };
        let Some(surface) = self.playlist_surface.as_mut() else {
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

    /// True when `id` is the equalizer window's id.
    fn is_eq_window(&self, id: WindowId) -> bool {
        self.eq_id == Some(id)
    }
    /// Creates or closes the dialog OS window so it matches the app's open
    /// dialog ([`App::dialog_open_kind`]).
    ///
    /// Creation needs the event loop, so this runs wherever the loop is
    /// available (`resumed`, `about_to_wait`). A failed creation is reported
    /// and skipped: the main window is unaffected. The window is left at the
    /// OS default position (cascading it off the main window is open).
    fn sync_dialog_window(&mut self, event_loop: &ActiveEventLoop) {
        let open = self.app.dialog_open_kind();
        if open == self.dialog_kind && open.is_some() {
            return;
        }
        if self.dialog_kind != open {
            self.dialog_window = None;
            self.dialog_surface = None;
            self.dialog_id = None;
            self.dialog_kind = None;
        }
        let Some(kind) = open else {
            return;
        };
        let (width, height) = kind.client_size();
        let attributes = Window::default_attributes()
            .with_title(kind.title())
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the dialog window: {error}");
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp: could not create the dialog softbuffer context: {error}");
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp: could not create the dialog softbuffer surface: {error}");
                return;
            }
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = surface.resize(width, height);
        }
        self.dialog_id = Some(window.id());
        self.dialog_window = Some(window);
        self.dialog_surface = Some(surface);
        self.dialog_kind = Some(kind);
        self.request_dialog_redraw();
    }

    /// True when `id` is the dialog window's id.
    fn is_dialog_window(&self, id: WindowId) -> bool {
        self.dialog_id == Some(id)
    }

    fn request_dialog_redraw(&self) {
        if let Some(window) = &self.dialog_window {
            window.request_redraw();
        }
    }

    /// Blits the dialog frame ([`App::render_dialog`]) to its surface.
    fn paint_dialog(&mut self) {
        let Some(kind) = self.dialog_kind else {
            return;
        };
        let image = self.app.render_dialog(kind);
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
            return;
        };
        let Some(surface) = self.dialog_surface.as_mut() else {
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

    /// A key press while the dialog window is focused.
    ///
    /// The dialog consumes `Enter` (confirm the jump box, the skin
    /// browser or the open-media dialog; a no-op on the load-playlist and preferences dialogs, whose
    /// confirmation is open per RFC 0010 and whose contents are unmeasured), `Escape` (close) and the keys each dialog types
    /// or moves with (digits for the jump box, arrows for the skin browser);
    /// the window closes itself when the app reports no open dialog
    /// afterwards.
    fn dialog_key_event(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        // Modified keys stay global (the same key-event path as the main
        // window); plain keys go to the dialog first.
        if modifiers != Modifiers::NONE {
            self.key_event(event);
            return;
        }
        if self.app.dialog_key(key) {
            self.sync_dialog_window(event_loop);
            self.request_dialog_redraw();
        }
    }

    /// Events for the dialog window.
    ///
    /// Closing the window closes the dialog (it never quits the player);
    /// the dialog has no buttons, so pointer input is ignored.
    fn dialog_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(kind) = self.dialog_kind {
                    self.app.close_dialog(kind);
                }
                self.sync_dialog_window(event_loop);
            }
            WindowEvent::RedrawRequested => self.paint_dialog(),
            WindowEvent::Resized(size) => {
                if let Some(kind) = self.dialog_kind {
                    let (width, height) = kind.client_size();
                    if (size.width != width || size.height != height)
                        && let Some(window) = &self.dialog_window
                    {
                        let _ = window.request_inner_size(PhysicalSize::new(width, height));
                    }
                }
                self.request_dialog_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.dialog_key_event(event_loop, &event);
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            _ => {}
        }
    }

    /// Window attributes for the main-menu popup at screen `origin`.
    ///
    /// The client size is the measured 323 x 569 (S04 §§1,8). The reference
    /// popup is a `#32768` system menu (settled, S04 §1); `winit` cannot set a
    /// window class, so this is an ordinary non-resizable window hosting the
    /// rendered menu model — the class itself is not reproduced.
    fn menu_window_attributes(origin: (i32, i32)) -> WindowAttributes {
        Window::default_attributes()
            .with_title("Rustamp Menu")
            .with_position(PhysicalPosition::new(origin.0, origin.1))
            .with_inner_size(PhysicalSize::new(
                MAIN_MENU_CLIENT_SIZE.0,
                MAIN_MENU_CLIENT_SIZE.1,
            ))
            .with_resizable(false)
    }

    /// Creates or closes the menu popup's OS window so it matches the app's
    /// popup state ([`App::main_menu_is_open`]).
    ///
    /// Creation needs the event loop, so this runs wherever the loop is
    /// available (`resumed`, `about_to_wait`, the menu close paths). A failed
    /// creation is reported and skipped: the main window is unaffected. The
    /// window opens anchored at the measured menu-button point
    /// ([`menu_popup_origin`]); it is not re-placed afterwards — popups do
    /// not follow the main window, and S04 §9 leaves placement unmeasured.
    fn sync_menu_window(&mut self, event_loop: &ActiveEventLoop) {
        if !self.app.main_menu_is_open() {
            if self.menu_window.is_some() {
                self.menu_window = None;
                self.menu_surface = None;
                self.menu_id = None;
            }
            return;
        }
        if self.menu_window.is_some() {
            return;
        }
        let main_outer = self
            .window
            .as_ref()
            .and_then(|window| window.outer_position().ok())
            .map(|pos| (pos.x, pos.y))
            .unwrap_or((DEFAULT_MAIN_POSITION.x, DEFAULT_MAIN_POSITION.y));
        let attributes = Self::menu_window_attributes(menu_popup_origin(main_outer));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the menu popup window: {error}");
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp: could not create the menu softbuffer context: {error}");
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp: could not create the menu softbuffer surface: {error}");
                return;
            }
        };
        if let (Some(width), Some(height)) = (
            NonZeroU32::new(MAIN_MENU_CLIENT_SIZE.0),
            NonZeroU32::new(MAIN_MENU_CLIENT_SIZE.1),
        ) {
            let _ = surface.resize(width, height);
        }
        self.menu_id = Some(window.id());
        self.menu_window = Some(window);
        self.menu_surface = Some(surface);
        self.request_menu_redraw();
    }

    /// True when `id` is the menu popup's id.
    fn is_menu_window(&self, id: WindowId) -> bool {
        self.menu_id == Some(id)
    }

    fn request_menu_redraw(&self) {
        if let Some(window) = &self.menu_window {
            window.request_redraw();
        }
    }

    /// Blits the menu frame ([`App::render_menu`]) to its surface.
    fn paint_menu(&mut self) {
        let image = self.app.render_menu();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
            return;
        };
        let Some(surface) = self.menu_surface.as_mut() else {
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

    /// A key press while the menu popup is focused.
    ///
    /// Own choice, stated plainly: S04 §9 leaves the OS popup's dismissal and
    /// keyboard behaviour unmeasured, so `Escape` closes the popup and every
    /// other key is ignored (no measured navigation to implement).
    fn menu_key_event(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        if key == Key::Escape {
            self.app.close_main_menu();
            self.sync_menu_window(event_loop);
        }
    }

    /// Events for the menu popup window, in popup-client coordinates.
    ///
    /// A left press on a row goes through [`App::menu_click`] (leaf rows
    /// activate through the key-dispatch path and close the popup; submenu
    /// parents open nothing and keep it open; separators are ignored); a leaf
    /// that opened a dialog is synced so the dialog window appears. Losing
    /// focus closes the popup — the click-outside dismissal, an own choice
    /// (S04 §9 unmeasured): a press anywhere else moves focus off first.
    /// Closing the window closes the popup (it never quits the player).
    fn menu_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.app.close_main_menu();
                self.sync_menu_window(event_loop);
            }
            WindowEvent::RedrawRequested => self.paint_menu(),
            WindowEvent::Resized(size) => {
                let (width, height) = MAIN_MENU_CLIENT_SIZE;
                if (size.width != width || size.height != height)
                    && let Some(window) = &self.menu_window
                {
                    let _ = window.request_inner_size(PhysicalSize::new(width, height));
                }
                self.request_menu_redraw();
            }
            WindowEvent::Focused(false) => {
                self.app.close_main_menu();
                self.sync_menu_window(event_loop);
            }
            WindowEvent::Focused(true) => {}
            WindowEvent::KeyboardInput { event, .. } => {
                self.menu_key_event(event_loop, &event);
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.menu_cursor = (position.x as i32, position.y as i32);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.menu_cursor;
                if let Err(error) = self.app.menu_click(x, y) {
                    eprintln!("rustamp: menu click failed: {error}");
                }
                self.sync_menu_window(event_loop);
                // A leaf may have opened a dialog or flipped a check mark.
                self.sync_dialog_window(event_loop);
                self.request_dialog_redraw();
                self.request_menu_redraw();
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Creates the playlist editor window with the same shell as the main
    /// window (S06, S01 §4): a non-resizable `winit` window with its own
    /// `softbuffer` surface, sized to the spec'd 275 x 116 client area
    /// ([`PLAYLIST_WINDOW_SIZE`]), under the same [`ensure_gui_allowed`] rail
    /// (checked once in [`run`], before either window exists).
    fn open_playlist_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.playlist_window.is_some() {
            return;
        }
        let (width, height) = PLAYLIST_WINDOW_SIZE;
        let attributes = Window::default_attributes()
            .with_title("Rustamp Playlist")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the playlist window: {error}");
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp: could not create the playlist softbuffer context: {error}");
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp: could not create the playlist softbuffer surface: {error}");
                return;
            }
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = surface.resize(width, height);
        }
        self.playlist_window = Some(window);
        self.playlist_surface = Some(surface);
        self.sync_playlist_position();
        self.request_playlist_redraw();
    }

    /// Keeps the playlist window docked: its screen position is the main
    /// window's position plus the S05 docked offset (playlist origin minus the
    /// main default position), so it follows the main window's bottom edge
    /// through [`App::playlist_window_origin`] — including shade reflow.
    /// Best-effort: a window whose position cannot be read keeps its own.
    /// Undocking by dragging the playlist itself stays open (S05 §7.8).
    fn sync_playlist_position(&self) {
        let (Some(main), Some(playlist)) = (self.window.as_ref(), self.playlist_window.as_ref())
        else {
            return;
        };
        let Ok(main_pos) = main.outer_position() else {
            return;
        };
        let (ox, oy) = self.app.playlist_window_origin();
        let target = PhysicalPosition::new(
            main_pos.x + ox - DEFAULT_MAIN_POSITION.x,
            main_pos.y + oy - DEFAULT_MAIN_POSITION.y,
        );
        if playlist.outer_position().ok() != Some(target) {
            playlist.set_outer_position(target);
        }
    }

    /// Events for the playlist editor window, in playlist-window coordinates.
    ///
    /// Input routes to [`App::playlist_mouse_down`] /
    /// [`App::playlist_mouse_up`]; keyboard shortcuts stay global (the same
    /// key-event path as the main window — S06 leaves playlist keys open).
    /// Closing the playlist keeps it open: hiding and reopening are open
    /// (S05 §7.8), so this records a note instead of quitting the player.
    fn playlist_event(&mut self, _event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                eprintln!("rustamp: playlist close is not implemented yet; the window stays open");
                self.request_playlist_redraw();
            }
            WindowEvent::RedrawRequested => self.paint_playlist(),
            WindowEvent::Resized(size) => {
                // The playlist never double-sizes (S05 §2.1): its client area
                // is always exactly the spec'd 275 x 116.
                let (width, height) = PLAYLIST_WINDOW_SIZE;
                if (size.width != width || size.height != height)
                    && let Some(window) = &self.playlist_window
                {
                    let _ = window.request_inner_size(PhysicalSize::new(width, height));
                }
                self.request_playlist_redraw();
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                let (width, height) = PLAYLIST_WINDOW_SIZE;
                let _ = inner_size_writer.request_inner_size(PhysicalSize::new(width, height));
                self.request_playlist_redraw();
            }
            WindowEvent::Focused(_) => {
                // Per-window activation is open, so the playlist shares the
                // main focus flag (see `App::render_playlist`).
                self.request_playlist_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                // No double-size scale: the playlist ignores it (S05 §2.1).
                self.playlist_cursor = (position.x as i32, position.y as i32);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.playlist_cursor;
                // Drags are open (S06 Q6: reorder vs selection block), so only
                // the press/release pair is routed; moves are not forwarded.
                let result = match state {
                    ElementState::Pressed => self.app.playlist_mouse_down(x, y),
                    ElementState::Released => self.app.playlist_mouse_up(x, y),
                };
                if let Err(error) = result {
                    eprintln!("rustamp: playlist pointer input failed: {error}");
                }
                self.sync_playlist_position();
                self.request_playlist_redraw();
            }
            _ => {}
        }
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
        if let Some(window) = &self.window {
            self.main_id = Some(window.id());
        }
        self.sync_window_state();
        self.sync_eq_window(event_loop);
        self.request_redraw();
        self.request_redraw_eq();
        self.sync_dialog_window(event_loop);
        self.request_dialog_redraw();
        self.sync_menu_window(event_loop);
        self.request_menu_redraw();
        self.open_playlist_window(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // The equalizer window routes to the EQ machine, the playlist window
        // to the playlist machine, the dialog window to the dialog manager;
        // everything else is the main window's.
        if self.is_eq_window(id) {
            self.eq_window_event(event_loop, event);
            return;
        }
        if self.is_dialog_window(id) {
            self.dialog_event(event_loop, event);
            return;
        }
        if self.is_menu_window(id) {
            self.menu_event(event_loop, event);
            return;
        }
        if self
            .playlist_window
            .as_ref()
            .is_some_and(|window| window.id() == id)
        {
            self.playlist_event(event_loop, event);
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.app.request_quit();
                event_loop.exit();
            }
            WindowEvent::Moved(position) => {
                // A docked neighbour follows the main window by the same delta
                // (S05 §4.2): the model moves through the stack geometry, then
                // the EQ window is re-placed below the live outer frame.
                if let Some(last) = self.last_main_outer {
                    self.stack.move_main(
                        position.x.saturating_sub(last.x),
                        position.y.saturating_sub(last.y),
                    );
                }
                self.last_main_outer = Some(position);
                self.place_eq_below_main();
                self.sync_playlist_position();
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Resized(size) => {
                let (width, height) = self.app.frame_size();
                if size.width < width || size.height < height {
                    // Never settle below the layout (the DPI-change floor).
                    self.applied_client = None;
                    self.sync_window_state();
                }
                // A main resize (windowshade, double-size) re-flows the stack
                // (S05 §3.3, §5): the equalizer tracks the new bottom edge,
                // and so does the playlist.
                self.place_eq_below_main();
                self.sync_playlist_position();
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
                // A press anywhere else dismisses the open menu popup instead
                // of reaching the main window. Own choice, stated plainly:
                // S04 §9 leaves the OS popup's dismissal unmeasured.
                if state == ElementState::Pressed && self.app.main_menu_is_open() {
                    self.left_down = false;
                    self.app.close_main_menu();
                    self.sync_menu_window(event_loop);
                    return;
                }
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
        self.sync_eq_window(event_loop);
        // A main resize (shade, double-size) moves the docked neighbour with
        // it (S05 §3.3, §5).
        self.place_eq_below_main();
        self.request_redraw();
        self.request_redraw_eq();
        // Dialogs follow the app's open dialog (opened by menu or shortcut,
        // closed by Escape, Enter or the window's close button).
        self.sync_dialog_window(event_loop);
        self.request_dialog_redraw();
        // The menu popup follows the app's popup state (opened by the
        // window-menu button or Alt+F, closed by selection, Escape, a press
        // elsewhere or its own close box).
        self.sync_menu_window(event_loop);
        self.request_menu_redraw();
        // The docked playlist tracks the main window (moves and shade reflow).
        self.sync_playlist_position();
        self.request_playlist_redraw();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }
}

impl Windowed {
    /// Events for the equalizer window (S07): input goes to the EQ machine,
    /// closing it toggles the window off, and resizes never settle below the
    /// layout — the same rails as the main window.
    fn eq_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // Closing the equalizer window toggles it off (the main
                // window's close still quits the player).
                if let Err(error) = self
                    .app
                    .submit(Command::ToggleWindow(CoreWindowId::Equalizer))
                {
                    eprintln!("rustamp: equalizer close failed: {error}");
                }
                self.sync_eq_window(event_loop);
                self.request_redraw_eq();
            }
            WindowEvent::RedrawRequested => self.paint_eq(),
            WindowEvent::Resized(size) => {
                let (width, height) = self.app.eq_frame_size();
                if size.width < width || size.height < height {
                    self.applied_eq_client = None;
                    self.sync_eq_window(event_loop);
                }
                self.request_redraw_eq();
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                let (width, height) = self.app.eq_frame_size();
                let _ = inner_size_writer.request_inner_size(PhysicalSize::new(width, height));
                self.applied_eq_client = Some((width, height));
                self.request_redraw_eq();
            }
            WindowEvent::Focused(focused) => {
                self.app.set_eq_focused(focused);
                self.request_redraw_eq();
            }
            WindowEvent::KeyboardInput { event, .. } => self.eq_key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.eq_cursor = self.to_eq_layout(position);
                if self.eq_left_down {
                    let (x, y) = self.eq_cursor;
                    if let Err(error) = self.app.eq_mouse_move(x, y) {
                        eprintln!("rustamp: equalizer pointer move failed: {error}");
                    }
                    self.request_redraw_eq();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.eq_cursor;
                let result = match state {
                    ElementState::Pressed => {
                        self.eq_left_down = true;
                        self.app.eq_mouse_down(x, y)
                    }
                    ElementState::Released => {
                        self.eq_left_down = false;
                        self.app.eq_mouse_up(x, y)
                    }
                };
                if let Err(error) = result {
                    eprintln!("rustamp: equalizer pointer input failed: {error}");
                }
                // A press may have armed a held sprite; a release may have
                // toggled double-size through the main window.
                self.sync_window_state();
                self.sync_eq_window(event_loop);
                self.request_redraw_eq();
            }
            _ => {}
        }
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

    /// Opens the real menu popup window. **VM-only, `#[ignore]`d by default,
    /// and gated on the test VM.**
    ///
    /// GUI must never run on the host. Run this only inside the headless test
    /// VM (the only host for anything graphical):
    ///
    /// ```text
    /// cargo test -p rustamp-app -- --ignored menu_popup
    /// ```
    ///
    /// Outside the test VM it refuses before any window exists
    /// ([`require_test_vm`]).
    #[test]
    #[ignore = "VM-only: opens the menu popup window; run inside the headless test VM"]
    #[allow(deprecated)]
    fn the_menu_popup_opens_at_the_measured_size() {
        // The gate, before any window exists.
        require_test_vm();

        let event_loop = EventLoop::new().expect("event loop");
        // Deprecated on purpose: outside `run_app` there is no
        // `ActiveEventLoop`, and this VM-only test just needs one window.
        let window = event_loop
            .create_window(Windowed::menu_window_attributes(menu_popup_origin((
                100, 100,
            ))))
            .expect("menu popup window");
        let size = window.inner_size();
        assert_eq!(
            (size.width, size.height),
            MAIN_MENU_CLIENT_SIZE,
            "the popup client area must be the measured 323 x 569 (S04 §§1,8)"
        );
    }
}
