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
//! tests that do open windows are `#[ignore]`d, VM-only, and refuse
//! unless they run inside the test VM
//! (`crates/rustamp-app/tests/headless.rs`). See `crates/rustamp-app/README.md`.
//!
//! **A spawned child must not create a window either — a console window counts
//! as a window on the host.** Test and tool code that spawns a process passes
//! `CREATE_NO_WINDOW`; `tests/headless.rs` centralises that in `no_window`.
//!
//! No test calls [`run`] outside those ignored, VM-gated cases.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustamp_core::command::Command;
use rustamp_core::state::WindowId as CoreWindowId;
use rustamp_platform::{HotkeyAction, init_dpi_awareness, single_instance, take_pending_hotkeys};
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

/// Builds the `winit` event loop, with an explicit off-main-thread opt-in.
///
/// `any_thread` selects `EventLoopBuilderExtWindows::with_any_thread` on
/// Windows (the function the `winit` panic itself names; elsewhere the flag
/// does not exist and the argument is ignored). Pass `true` only from the
/// VM-gated window tests: the test harness runs every test on a worker
/// thread, so a test that builds a loop cannot be on the main thread, and
/// `EventLoop::new()` would panic there. Every other caller — in particular
/// the shipped player in [`run`], which owns the main thread — passes
/// `false`, which is exactly `EventLoop::new()` (`winit` implements `new()`
/// as `builder().build()` with the flag at its default off), so the product
/// keeps the cross-platform main-thread guarantee the panic protects.
///
/// Why an explicit parameter and not an environment variable: an env var is
/// process-global state invisible at the construction site — a leaked
/// variable would silently flip the product into the hazardous mode. A
/// parameter names the mode at every call site, and the default call reads
/// as today's behaviour. This does not weaken the GUI rails: the tests still
/// call [`require_test_vm`] before anything graphical exists, and
/// [`ensure_gui_allowed`] still refuses the product off-VM.
pub fn build_event_loop(any_thread: bool) -> Result<EventLoop<()>, winit::error::EventLoopError> {
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows as _;
        builder.with_any_thread(any_thread);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = any_thread;
    }
    builder.build()
}

/// Opens the main window (plus the equalizer window when it is open and the
/// playlist window) and runs until the main window is closed.
///
/// Blocks and must run on the main thread. Never called by a test, except the
/// `#[ignore]`d, VM-only ones. Returns an error without opening anything
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
    // The product owns the main thread, so the default mode: exactly
    // `EventLoop::new()`. The off-main-thread opt-in (`true`) exists only
    // for the VM-gated window tests, which run on harness worker threads.
    let event_loop = build_event_loop(false)?;
    let run_res = event_loop.run_app(&mut presenter);
    if run_res.is_ok() {
        presenter.sync_window_layouts_to_app();
        if let Err(error) = presenter.app.save_config() {
            eprintln!("rustamp: {error}");
        }
    }
    run_res.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

struct Windowed {
    app: App,
    /// Declared before the window/surfaces so its Drop removes the icon and
    /// callback while the owning HWND is still alive (F3/F5).
    #[cfg(windows)]
    tray: Option<rustamp_platform::TrayIcon>,
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
    last_playlist_click: Option<(Instant, (i32, i32))>,
    modifiers: ModifiersState,
    /// The client size last requested, so repeat requests are skipped.
    applied_client: Option<(u32, u32)>,
    /// The equalizer's client size last requested.
    applied_eq_client: Option<(u32, u32)>,
    /// The always-on-top level last applied to the OS window.
    applied_on_top: bool,
    /// Runtime show/hide state last applied to the main OS window.
    applied_main_visible: bool,
    /// The main window title last applied ([`App::window_title`]).
    applied_title: Option<String>,
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
            #[cfg(windows)]
            tray: None,
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
            last_playlist_click: None,
            modifiers: ModifiersState::empty(),
            applied_client: None,
            applied_eq_client: None,
            applied_on_top,
            applied_main_visible: true,
            applied_title: None,
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
            if self.app.main_visible() != self.applied_main_visible {
                self.applied_main_visible = self.app.main_visible();
                window.set_visible(self.applied_main_visible);
                if self.applied_main_visible {
                    window.set_minimized(false);
                    window.focus_window();
                }
            }
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
            let mut attributes = Window::default_attributes()
                .with_title("Rustamp Equalizer")
                .with_inner_size(PhysicalSize::new(width, height))
                .with_resizable(false);
            let eq_layout = self
                .app
                .config()
                .ui
                .windows
                .iter()
                .find(|w| w.id == CoreWindowId::Equalizer);
            if let Some(layout) = eq_layout {
                attributes = attributes.with_position(PhysicalPosition::new(layout.x, layout.y));
            }
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
            if eq_layout.is_none() {
                self.place_eq_below_main();
            }
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

    /// Synchronizes current live OS window positions, visibility, and shade states
    /// into the app configuration so they are preserved on exit.
    pub(crate) fn sync_window_layouts_to_app(&mut self) {
        if let Some(window) = &self.window
            && let Ok(pos) = window.outer_position()
        {
            self.app
                .set_window_position(CoreWindowId::Main, pos.x, pos.y);
        }
        self.app
            .set_window_shaded(CoreWindowId::Main, self.app.state().windowshade);
        self.app.set_window_visible(CoreWindowId::Main, true);

        let eq_visible = self.app.eq_open();
        self.app
            .set_window_visible(CoreWindowId::Equalizer, eq_visible);
        if let Some(eq) = &self.eq_window
            && let Ok(pos) = eq.outer_position()
        {
            self.app
                .set_window_position(CoreWindowId::Equalizer, pos.x, pos.y);
        }

        let pl_visible = self.app.playlist_open();
        self.app
            .set_window_visible(CoreWindowId::Playlist, pl_visible);
        if let Some(pl) = &self.playlist_window
            && let Ok(pos) = pl.outer_position()
        {
            self.app
                .set_window_position(CoreWindowId::Playlist, pos.x, pos.y);
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

    /// A key press while the playlist window is focused (S03 §2: the playlist
    /// table is checked before the global one). `App::playlist_key_down` owns
    /// the table's own keys (Delete, Ctrl+A, Ctrl+I) and falls through to the
    /// shared path for everything else, so nothing regresses.
    fn playlist_key_event(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        match self.app.playlist_key_down(key, modifiers) {
            Ok(Some(ShortcutOutcome::Commands(commands))) => {
                println!("rustamp: playlist shortcut {key:?} -> {commands:?}");
            }
            Ok(Some(ShortcutOutcome::Handled)) => {
                println!("rustamp: playlist shortcut {key:?} handled locally");
            }
            Ok(Some(ShortcutOutcome::Unhandled(action))) => {
                println!(
                    "rustamp: playlist shortcut {key:?} -> {action:?} (command id {}, no Command yet)",
                    action.command_id()
                );
            }
            Ok(None) => return,
            Err(error) => eprintln!("rustamp: playlist shortcut dispatch failed: {error}"),
        }
        self.sync_window_state();
        self.request_redraw();
        self.request_playlist_redraw();
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

    /// Applies [`App::window_title`] to the main window when it changed.
    fn sync_title(&mut self) {
        let title = self.app.window_title();
        if self.applied_title.as_deref() != Some(title.as_str())
            && let Some(window) = &self.window
        {
            window.set_title(&title);
            self.applied_title = Some(title);
        }
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
    /// confirmation is open per RFC 0010 and whose contents are unmeasured), `Escape` (close),
    /// `Backspace`, typed text (the open-media field and the jump box, see
    /// [`App::dialog_text`]) and the arrows of the skin browser; the window
    /// closes itself when the app reports no open dialog afterwards.
    fn dialog_key_event(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        // A dialog with a text field takes the OS text of the press first:
        // it is layout-aware and carries case and punctuation (`:`, `\`,
        // `.`, AltGr characters), so an ordinary path can be typed (BETA-03).
        // Control characters (Enter, Backspace, Escape, Ctrl+letter) are not
        // text and fall through to the key path below. Ctrl or Alt alone is a
        // shortcut, not text; both together are AltGr on Windows (the
        // backslash on a German layout), which is text.
        let shortcut_chord = self.modifiers.control_key() != self.modifiers.alt_key();
        if !shortcut_chord
            && self.app.dialog_accepts_text()
            && let Some(text) = event.text.as_deref()
            && self.app.dialog_text(text)
        {
            self.request_dialog_redraw();
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
            // A confirmed open-media dialog adds (and may start) a track:
            // the playlist and the main window show it at once.
            self.request_playlist_redraw();
            self.request_redraw();
        }
    }

    /// Events for the dialog window.
    ///
    /// Closing the window closes the dialog (it never quits the player);
    /// the dialog has no buttons, so pointer input is ignored. A file dropped
    /// from the shell onto an open-media dialog becomes a pick
    /// ([`App::dialog_drop_file`]; `winit` delivers one event per file).
    fn dialog_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::DroppedFile(path) => {
                if self.app.dialog_drop_file(path) {
                    self.request_dialog_redraw();
                }
            }
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

    /// Creates or drops the playlist editor's OS window so it matches
    /// [`App::playlist_open`], the way [`Windowed::sync_eq_window`] follows
    /// the equalizer: the PL button, the menu row, the playlist's own Close
    /// and its OS close box all flip that one flag.
    fn sync_playlist_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.playlist_open() {
            self.open_playlist_window(event_loop);
        } else if self.playlist_window.is_some() {
            self.playlist_surface = None;
            self.playlist_window = None;
            self.last_playlist_click = None;
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
        let mut attributes = Window::default_attributes()
            .with_title("Rustamp Playlist")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let pl_layout = self
            .app
            .config()
            .ui
            .windows
            .iter()
            .find(|w| w.id == CoreWindowId::Playlist);
        if let Some(layout) = pl_layout {
            attributes = attributes.with_position(PhysicalPosition::new(layout.x, layout.y));
        }
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
    /// Closing the playlist hides it ([`App::close_playlist_window`], the
    /// same toggle as its skinned Close and the PL button) and never quits
    /// the player; the PL button brings it back.
    fn playlist_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Err(error) = self.app.close_playlist_window() {
                    eprintln!("rustamp: playlist close failed: {error}");
                }
                self.sync_playlist_window(event_loop);
                self.request_redraw();
            }
            WindowEvent::Moved(position) => {
                self.app
                    .set_window_position(CoreWindowId::Playlist, position.x, position.y);
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
            WindowEvent::KeyboardInput { event, .. } => self.playlist_key_event(&event),
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
                    ElementState::Pressed => {
                        let now = Instant::now();
                        let is_double =
                            if let Some((last_time, (lx, ly))) = self.last_playlist_click {
                                now.duration_since(last_time) < Duration::from_millis(500)
                                    && (x - lx).abs() <= 4
                                    && (y - ly).abs() <= 4
                            } else {
                                false
                            };
                        if is_double {
                            self.last_playlist_click = None;
                            self.app.playlist_double_click(x, y)
                        } else {
                            self.last_playlist_click = Some((now, (x, y)));
                            self.app.playlist_mouse_down(x, y)
                        }
                    }
                    ElementState::Released => self.app.playlist_mouse_up(x, y),
                };
                if let Err(error) = result {
                    eprintln!("rustamp: playlist pointer input failed: {error}");
                }
                // The title-bar Close may have hidden the editor, ADD may
                // have opened the open-media dialog.
                self.sync_playlist_window(event_loop);
                self.sync_dialog_window(event_loop);
                self.sync_playlist_position();
                self.request_playlist_redraw();
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Starts an OS window drag for a background press when easy-move is on
    /// ([`App::easy_drag_main`]). Returns whether the press was consumed by
    /// the drag, so the control path is skipped; a failed drag falls through
    /// to the controls so the press is never swallowed. Only the VM can prove
    /// it (no window may open here); the decision itself is headless-tested
    /// on [`App::easy_drag_main`].
    fn try_easy_drag(&mut self, x: i32, y: i32) -> Option<()> {
        if !self.app.easy_drag_main(x, y) {
            return None;
        }
        let window = self.window.as_ref()?;
        if let Err(error) = window.drag_window() {
            eprintln!("rustamp: easy-move drag failed: {error}");
            return None;
        }
        Some(())
    }

    /// F3's pending actions enter the same external command channel as a
    /// plugin or forwarded command, never the already-applied UI path.
    fn submit_hotkeys(&mut self, actions: impl IntoIterator<Item = HotkeyAction>) {
        for action in actions {
            self.submit_platform_commands(action.commands());
        }
    }

    fn submit_platform_commands(&mut self, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            if let Err(error) = self.app.submit(command) {
                eprintln!("rustamp: desktop command failed: {error}");
            }
        }
    }

    /// F3 attach seam: called once after the main window and surface exist.
    /// Failure leaves the window running, with no icon and no callback.
    #[cfg(windows)]
    fn attach_tray(&mut self) {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Some(window) = &self.window else {
            return;
        };
        match window.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => {
                    match rustamp_platform::TrayIcon::attach(handle.hwnd.get()) {
                        Ok(tray) => self.tray = Some(tray),
                        Err(error) => eprintln!("rustamp: tray icon unavailable: {error}"),
                    }
                }
                _ => eprintln!("rustamp: tray icon unavailable: expected a Win32 window handle"),
            },
            Err(error) => eprintln!("rustamp: tray icon unavailable: {error}"),
        }
    }

    /// The three tray seams, drained once per frame on the UI thread.
    ///
    /// The order is:
    /// 1. Shell-restart re-add: a just-restored icon is live before this
    ///    frame's clicks are considered.
    /// 2. Left click: primary direct action (window toggle) is drained before
    ///    the menu. If both a left and right click land in the same 16 ms frame,
    ///    the direct toggle is processed first rather than getting trapped
    ///    behind the modal popup loop.
    /// 3. Context menu: opens the modal Win32 popup.
    #[cfg(windows)]
    fn drain_tray(&mut self) {
        self.readd_tray_after_shell_restart();
        self.drain_tray_left_click();
        self.drain_tray_menu();
    }

    /// F5 follow-up, gap 2: Explorer restarted and dropped every icon. The
    /// callback queued the `TaskbarCreated` broadcast; re-add through the same
    /// `NIM_ADD` path `attach` uses. The subclass survived, so it is not
    /// reinstalled, and a refusal degrades to a warning — no retry loop.
    #[cfg(windows)]
    fn readd_tray_after_shell_restart(&mut self) {
        let Some(tray) = &mut self.tray else {
            return;
        };
        if !tray.take_shell_restart() {
            return;
        }
        if let Err(error) = tray.readd() {
            eprintln!("rustamp: tray icon lost after a shell restart: {error}");
        }
    }

    /// F5 follow-up, gap 1: a **left** click activates the window instead of
    /// doing nothing. It routes through the existing model entry
    /// [`rustamp_platform::TRAY_LEFT_CLICK`] and the same
    /// `submit_platform_commands` path a menu pick uses, so the tray still
    /// invents no command and no popup is opened here.
    #[cfg(windows)]
    fn drain_tray_left_click(&mut self) {
        let Some(tray) = &self.tray else {
            return;
        };
        if !tray.take_left_click() {
            return;
        }
        self.submit_platform_commands(rustamp_platform::TRAY_LEFT_CLICK.commands());
    }

    /// The native callback only queues. Open the popup here, outside winit's
    /// window procedure, then route the selected model entry through submit.
    #[cfg(windows)]
    fn drain_tray_menu(&mut self) {
        let Some(tray) = &self.tray else {
            return;
        };
        if !tray.take_menu_request() {
            return;
        }
        match tray.show_menu(rustamp_platform::TRAY_MENU) {
            Ok(Some(entry)) => self.submit_platform_commands(entry.commands()),
            Ok(None) => {} // Cancellation is not a failure or a command.
            Err(error) => eprintln!("rustamp: tray menu unavailable: {error}"),
        }
    }
}

impl ApplicationHandler for Windowed {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (width, height) = self.app.frame_size();
        let mut attributes = Window::default_attributes()
            .with_title("Rustamp")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_resizable(false);
        let main_layout = self
            .app
            .config()
            .ui
            .windows
            .iter()
            .find(|w| w.id == CoreWindowId::Main);
        if let Some(layout) = main_layout {
            attributes = attributes.with_position(PhysicalPosition::new(layout.x, layout.y));
        }
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
        #[cfg(windows)]
        self.attach_tray();
        self.sync_window_state();
        self.sync_eq_window(event_loop);
        self.request_redraw();
        self.request_redraw_eq();
        self.sync_dialog_window(event_loop);
        self.request_dialog_redraw();
        self.sync_menu_window(event_loop);
        self.request_menu_redraw();
        self.sync_playlist_window(event_loop);
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
                self.app
                    .set_window_position(CoreWindowId::Main, position.x, position.y);
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
                        // Easy-move (W10 finding F5): a press no control owns
                        // moves the window instead of reaching the machine.
                        match self.try_easy_drag(x, y) {
                            Some(()) => Ok(Vec::new()),
                            None => self.app.mouse_down(x, y),
                        }
                    }
                    ElementState::Released => {
                        self.left_down = false;
                        self.app.mouse_up(x, y)
                    }
                };
                if let Err(error) = result {
                    eprintln!("rustamp: pointer input failed: {error}");
                }
                // A click may have toggled double-size (the clutter-bar `D`),
                // shaded or minimized the window, or opened the open-media
                // dialog (Eject); a close is picked up in `about_to_wait`.
                self.sync_window_state();
                self.sync_dialog_window(event_loop);
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // S03 §5: the main menu, or the EQ/PL toggle on those buttons.
                if self.app.main_menu_is_open() {
                    self.app.close_main_menu();
                } else {
                    let (x, y) = self.cursor;
                    if let Err(error) = self.app.mouse_right_down(x, y) {
                        eprintln!("rustamp: right-click failed: {error}");
                    }
                }
                self.sync_menu_window(event_loop);
                self.sync_eq_window(event_loop);
                self.sync_playlist_window(event_loop);
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Command lines a second launch handed over (F2) are applied here, on
        // the UI thread, through the same command path the menu uses.
        for request in single_instance::take_pending() {
            for command in request.commands() {
                if let Err(error) = self.app.submit(command) {
                    eprintln!("rustamp: forwarded command failed: {error}");
                }
            }
        }
        // F3 seam, next to F2: every queued firing is submitted exactly once,
        // in queue order, before the usual pump/state synchronisation below.
        self.submit_hotkeys(take_pending_hotkeys());
        #[cfg(windows)]
        self.drain_tray();
        // A `--no-audio` run has no device pulling on its own thread, so the
        // virtual clock is driven here, once per frame, exactly as
        // `run_headless` drives it headless. With a real device this is a
        // no-op.
        self.app.drive_null_clock();
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
        // The playlist editor follows its open flag and, while open, tracks
        // the main window (moves and shade reflow).
        self.sync_playlist_window(event_loop);
        self.sync_playlist_position();
        self.request_playlist_redraw();
        // An error the user must see goes to the native title (BETA-04).
        self.sync_title();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Remove the icon/callback before winit tears down the owning HWND.
        #[cfg(windows)]
        {
            self.tray = None;
        }
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
            WindowEvent::Moved(position) => {
                self.app
                    .set_window_position(CoreWindowId::Equalizer, position.x, position.y);
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
        KeyCode::Backspace => Key::Backspace,
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
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;
    use rustamp_core::config::Config;
    use rustamp_core::playlist::MediaLocation;
    use rustamp_core::state::TransportState;
    use rustamp_platform::TrayMenuEntry;

    /// No event loop, window, hotkey registration, host process or audio device.
    fn desktop_test_presenter(with_tracks: bool) -> Windowed {
        let mut app = App::new(AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: None,
            seconds: None,
        })
        .unwrap();
        if with_tracks {
            let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../rustamp-decoders/tests/fixtures/sine.flac");
            assert!(fixture.exists(), "missing fixture {}", fixture.display());
            app.submit(Command::AddItems {
                items: vec![
                    MediaLocation::File(fixture.clone()),
                    MediaLocation::File(fixture),
                ],
                at: None,
                play: false,
            })
            .unwrap();
        }
        Windowed::new(app)
    }

    fn wait_for_index(presenter: &mut Windowed, index: u32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while presenter.app.engine().current_index() != Some(index) {
            presenter.app.pump().unwrap();
            assert!(Instant::now() < deadline, "track {index} was not selected");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_hotkey_drain_consumer_routes_in_order_through_the_real_app_channel() {
        let mut presenter = desktop_test_presenter(true);
        // Order matters: pause after play must leave the transport Paused.
        presenter.submit_hotkeys([HotkeyAction::Play, HotkeyAction::PauseUnpause]);
        assert_eq!(presenter.app.engine().transport(), TransportState::Paused);
        wait_for_index(&mut presenter, 0);
        presenter.submit_hotkeys([HotkeyAction::PauseUnpause]);
        assert_eq!(presenter.app.engine().transport(), TransportState::Playing);
        presenter.submit_hotkeys([HotkeyAction::NextTrack]);
        wait_for_index(&mut presenter, 1);
        presenter.submit_hotkeys([HotkeyAction::PreviousTrack]);
        wait_for_index(&mut presenter, 0);
        presenter.submit_hotkeys([HotkeyAction::Stop, HotkeyAction::ToggleMainWindow]);
        assert_eq!(presenter.app.engine().transport(), TransportState::Stopped);
        assert!(
            !presenter.app.main_visible(),
            "one toggle must hide, not toggle twice"
        );
        presenter.submit_hotkeys([]);
        assert!(
            !presenter.app.main_visible(),
            "an empty drain must do nothing"
        );
        assert!(presenter.window.is_none(), "the test must stay headless");
    }

    #[test]
    fn tray_picks_share_the_command_path_for_visibility_transport_and_quit() {
        let mut presenter = desktop_test_presenter(true);
        for (entry, expected) in [
            (TrayMenuEntry::Play, TransportState::Playing),
            (TrayMenuEntry::PauseUnpause, TransportState::Paused),
            (TrayMenuEntry::PauseUnpause, TransportState::Playing),
            (TrayMenuEntry::Stop, TransportState::Stopped),
        ] {
            presenter.submit_platform_commands(entry.commands());
            assert_eq!(presenter.app.engine().transport(), expected);
        }
        presenter.submit_platform_commands(TrayMenuEntry::ToggleMainWindow.commands());
        assert!(!presenter.app.main_visible());
        presenter.app.submit(Command::ShowMainWindow).unwrap();
        assert!(
            presenter.app.main_visible(),
            "the normal show command can recover it"
        );
        presenter.submit_platform_commands(TrayMenuEntry::ExitPlayer.commands());
        assert!(
            presenter.app.quit(),
            "Exit must reach the app, not just close a popup"
        );
        assert!(presenter.window.is_none());
    }

    /// F5 follow-up, gap 1: the left click reaches show/hide through the real
    /// App command channel, both ways, and stays distinct from a right click.
    #[test]
    fn a_tray_left_click_toggles_the_main_window_through_the_command_path() {
        let mut presenter = desktop_test_presenter(false);
        assert!(presenter.app.main_visible(), "visibility starts true");
        presenter.submit_platform_commands(rustamp_platform::TRAY_LEFT_CLICK.commands());
        assert!(!presenter.app.main_visible(), "a left click hides it");
        presenter.submit_platform_commands(rustamp_platform::TRAY_LEFT_CLICK.commands());
        assert!(presenter.app.main_visible(), "and shows it again");
        // Same model entry as the menu's show/hide row, so the two agree.
        presenter.submit_platform_commands(TrayMenuEntry::ToggleMainWindow.commands());
        assert!(!presenter.app.main_visible());
        presenter.submit_platform_commands(rustamp_platform::TRAY_LEFT_CLICK.commands());
        assert!(presenter.app.main_visible());
        // A left click is not a right click: it never quits and never moves
        // the transport, whatever the popup rows would have done.
        assert!(!presenter.app.quit());
        assert_eq!(presenter.app.engine().transport(), TransportState::Stopped);
        // And a right-click pick that is *not* show/hide leaves visibility be.
        presenter.submit_platform_commands(TrayMenuEntry::Stop.commands());
        assert!(
            presenter.app.main_visible(),
            "a right-click transport pick must not toggle the window"
        );
        assert!(presenter.window.is_none(), "the test must stay headless");
    }

    #[test]
    fn a_refused_play_on_an_empty_queue_does_not_drop_the_next_desktop_action() {
        let mut presenter = desktop_test_presenter(false);
        presenter.submit_hotkeys([HotkeyAction::Play, HotkeyAction::ToggleMainWindow]);
        assert_eq!(presenter.app.engine().transport(), TransportState::Stopped);
        assert!(!presenter.app.main_visible());
        presenter.submit_hotkeys([HotkeyAction::ToggleMainWindow]);
        assert!(presenter.app.main_visible());
    }

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

        // Off the main thread on purpose: unit tests run on harness worker
        // threads, so `EventLoop::new()` would panic here. See
        // [`build_event_loop`].
        let event_loop = build_event_loop(true).expect("event loop");
        // Deprecated on purpose: outside `run_app` there is no
        // `ActiveEventLoop`, and this VM-only test just needs one window.
        let origin = menu_popup_origin((100, 100));
        let window = event_loop
            .create_window(Windowed::menu_window_attributes(origin))
            .expect("menu popup window");
        let size = window.inner_size();
        assert_eq!(
            (size.width, size.height),
            MAIN_MENU_CLIENT_SIZE,
            "the popup client area must be the measured 323 x 569 (S04 §§1,8)"
        );
        let pos = window.outer_position().expect("read the popup position");
        assert_eq!(
            (pos.x, pos.y),
            origin,
            "the popup must open where it was asked to (probe-only anchor; S04 §9 leaves placement unmeasured)"
        );
    }
}
