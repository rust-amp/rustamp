// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The windowed presenter: `winit` + `softbuffer` shells around [`App`].
//!
//! The app owns the OS and the clock (task card N1): this module creates the
//! windows, forwards OS input into [`App`], pumps the app on a ~60 Hz timer so
//! the engine's events reach the UI machine and the marquee/time animate, and
//! blits the frames [`App::render`] and [`App::render_eq`] produce.
//!
//! The **main menu is not one of these windows**: it is the operating
//! system's own popup ([`Windowed::drain_main_menu`]), because the reference
//! menus are system menus (S04 §1: class `#32768`).
//!
//! All three classic windows take their positions from
//! [`rustamp_ui::docking::DockedStack`]. Coincident outside edges connect them;
//! moving the main carries its connected neighbours and resizing reflows them.
//! EQ/playlist title drags can detach and snap to visible neighbours. Snapping
//! uses `UiConfig::snap_distance` (0 disables it). These snap/undock policies
//! and restoring connections from saved geometry are Rustamp choices: S05
//! leaves tolerance, participating edges and persistence open.
//!
//! # The window frame: there is none (wave122)
//!
//! Every window here is **borderless**: the title bar is part of the skin
//! (S01 draws it at (0,0,275,14) inside the client area, with the skin's own
//! close and shade buttons), so a native caption would be a second title bar
//! with the wrong buttons, the wrong height and the wrong colours - and it
//! would be what the user drags instead of the skinned bar. `winit` defaults
//! to a decorated window, so saying so is required: every window below is
//! built by [`Windowed::window_attributes`], which delegates to the one
//! shared shell [`rustamp_ui::shell::skinned_window_attributes`]. This module
//! never calls `Window::default_attributes()` itself, and
//! `crates/rustamp-app/tests/window_shell.rs` fails if it starts to.
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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustamp_core::command::{Command, OpenMediaKind, PrefsPage};
use rustamp_core::state::WindowId as CoreWindowId;
use rustamp_platform::{HotkeyAction, init_dpi_awareness, single_instance, take_pending_hotkeys};
use rustamp_ui::shell::TaskbarRole;
use rustamp_ui::{
    DEFAULT_MAIN_POSITION, DialogKind, Key, Modifiers, PLAYLIST_WINDOW_SIZE, ShortcutOutcome,
    docking::{ClassicWindow, DockedStack, WindowKind, WindowPosition},
    menu::menu_popup_origin,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use crate::app::App;

/// A window this presenter creates.
///
/// It exists for the decisions that belong to the *window* rather than to the
/// creation site: today the taskbar role. Enumerating them in one type is
/// what lets a headless test state "exactly one window is the application"
/// instead of trusting five call sites to agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenterWindow {
    /// The main window (S01, S02).
    Main,
    /// The equalizer window (S05, S07).
    Equalizer,
    /// The playlist editor (S06).
    Playlist,
    /// One of the dialog frames ([`DialogKind`]).
    Dialog,
}

impl PresenterWindow {
    /// Every window of this presenter, for the tests that must cover all of
    /// them.
    #[cfg(test)]
    const ALL: [PresenterWindow; 4] = [
        PresenterWindow::Main,
        PresenterWindow::Equalizer,
        PresenterWindow::Playlist,
        PresenterWindow::Dialog,
    ];

    /// Whether this window stands for the player in the taskbar.
    ///
    /// Only the main window does: the player is one application however many
    /// of its windows are open, and the taskbar must say so
    /// ([`rustamp_ui::shell::TaskbarRole`]). The main window is the right
    /// one, because it lives for the whole run - closing it quits - so the
    /// entry never vanishes while the player is still there.
    const fn taskbar_role(self) -> TaskbarRole {
        match self {
            PresenterWindow::Main => TaskbarRole::Application,
            PresenterWindow::Equalizer | PresenterWindow::Playlist | PresenterWindow::Dialog => {
                TaskbarRole::Secondary
            }
        }
    }
}

/// How the presenter satisfies a dialog the app opened.
///
/// The app's dialog model is headless: a command *requests* a dialog, and the
/// presenter decides how the OS asks the user. Keeping that decision in one
/// pure function is what lets a headless test state which dialogs are native
/// - no window, no desktop needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogRoute {
    /// The standard Windows file-open dialog, with multiple selection
    /// allowed; `play` is the requesting command's flag, carried through to
    /// the [`Command::AddItems`] the choice produces.
    NativeFilePicker { play: bool },
    /// The preferences dialog: a real Windows dialog (tree of pages, group
    /// boxes, check boxes) opened on `page`.
    NativePreferences { page: PrefsPage },
    /// A window of ours, painted from the dialog model.
    OwnWindow,
}

impl DialogRoute {
    /// Which route `kind` takes.
    ///
    /// Only the **files** face of the open-media dialog is native: Windows
    /// has a file-open dialog for exactly that, and asking the user to type a
    /// path into a window of ours instead is the defect. A directory pick and
    /// a location/URL entry are not file-open dialogs, and everything else
    /// (About, file info, jump, the skin browser, load playlist)
    /// is the player's own face with no OS equivalent.
    fn for_dialog(kind: DialogKind) -> Self {
        match kind {
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play,
            } => DialogRoute::NativeFilePicker { play },
            DialogKind::Preferences(page) => DialogRoute::NativePreferences { page },
            _ => DialogRoute::OwnWindow,
        }
    }
}

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
    let mut app = app;
    apply_first_start_scale(&mut app);
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

/// The effective DPI from which the player starts at double size.
///
/// 144 is 150 %: below it the 275x116 classic window is still a sensible size,
/// from there on it is too small to read.
const HIGH_DPI: u32 = 144;

/// Whether a start with no saved configuration should begin at double size:
/// only on the first start (nothing saved yet) and only on a high-DPI primary
/// display. Once the config file exists the saved `ui.double_size` (and the
/// preferences) decide, so the user's choice is never overridden.
fn first_start_double_size(config_exists: bool, primary_dpi: Option<u32>) -> bool {
    !config_exists && primary_dpi.is_some_and(|dpi| dpi >= HIGH_DPI)
}

/// Applies [`first_start_double_size`] to `app`. Runs after DPI awareness is
/// set, so the monitor DPI is the real one and not the virtualized 96.
fn apply_first_start_scale(app: &mut App) {
    let Some(path) = app.config_path() else {
        return;
    };
    let exists = path.exists();
    let dpi = rustamp_platform::DisplaySnapshot::capture()
        .ok()
        .and_then(|snapshot| snapshot.primary().map(|monitor| monitor.dpi));
    if first_start_double_size(exists, dpi)
        && !app.state().double_size
        && let Err(error) = app.submit(Command::ToggleDoubleSize)
    {
        eprintln!("rustamp: could not start at double size: {error}");
    }
}

struct Windowed {
    app: App,
    /// Removed before the owning HWND; native callbacks never borrow App.
    #[cfg(windows)]
    external_control: Option<crate::external_control::native::ExternalControl>,
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
    /// through the S05 geometry (see [`Windowed::apply_docking_positions`]).
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
    /// A standard file-open dialog waiting to be shown, with the `play` flag
    /// of the request that asked for it.
    ///
    /// The native dialog is modal and runs its own message loop, so it must
    /// not be opened from inside a window procedure - the same rule the tray
    /// popup follows. The request is recorded here and drained once per tick
    /// in [`Windowed::drain_media_pick`].
    pending_media_pick: Option<(DialogKind, bool)>,
    /// The preferences page the app asked for, opened from the tick for the
    /// same reason as the file pick (never from inside a window procedure).
    pending_prefs: Option<PrefsPage>,
    /// The native preferences dialog while it is open.
    #[cfg(windows)]
    prefs_window: Option<rustamp_platform::PrefsWindow>,
    cursor: (i32, i32),
    eq_cursor: (i32, i32),
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
    /// Authoritative physical geometry, including retained positions of closed
    /// windows. Exact edge connections are reconstructed on restart.
    stack: DockedStack,
}

impl Windowed {
    fn new(app: App) -> Self {
        let applied_on_top = app.always_on_top();
        let stack = Self::restore_stack(&app);
        Self {
            app,
            #[cfg(windows)]
            external_control: None,
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
            pending_media_pick: None,
            pending_prefs: None,
            #[cfg(windows)]
            prefs_window: None,
            cursor: (0, 0),
            eq_cursor: (0, 0),
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
            stack,
        }
    }

    fn restore_stack(app: &App) -> DockedStack {
        let mut stack = DockedStack::default_launch();
        stack.set_main_shaded(app.state().windowshade);
        stack.set_main_double_size(app.state().double_size);
        stack.set_equalizer_double_size(app.eq_state().double_size);
        stack.set_equalizer_shaded(app.eq_state().windowshade);
        if let Some(layout) = app
            .config()
            .ui
            .windows
            .iter()
            .find(|w| w.id == CoreWindowId::Main)
        {
            stack.move_main(
                layout.x.saturating_sub(stack.main.x),
                layout.y.saturating_sub(stack.main.y),
            );
        }
        for (id, window) in [
            (CoreWindowId::Equalizer, &mut stack.equalizer),
            (CoreWindowId::Playlist, &mut stack.playlist),
        ] {
            if let Some(layout) = app.config().ui.windows.iter().find(|w| w.id == id) {
                window.x = layout.x;
                window.y = layout.y;
            }
        }
        stack
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
        if self.app.eq_state().double_size != self.stack.equalizer.double_size {
            self.stack
                .set_equalizer_double_size(self.app.eq_state().double_size);
        }
        if self.app.eq_state().windowshade != self.stack.equalizer.shaded {
            self.stack
                .set_equalizer_shaded(self.app.eq_state().windowshade);
        }
        self.apply_docking_positions();
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

    /// Applies the window-management requests (`ShowMainWindow`, S03 id
    /// 40258; `FocusNextWindow`, S03 id 40063): un-minimize first, then move
    /// keyboard focus. Both requests are taken every call, so none outlives
    /// the frame it was made in. A focus request naming a window with no OS
    /// window (a closed equalizer, a windowless id) is dropped: a closed
    /// window cannot take focus, and the headless model already skipped it.
    /// Focusing never resizes, so shade and double-size geometry is untouched.
    fn apply_focus_requests(&mut self) {
        let restore = self.app.take_restore_request();
        let target = self.app.take_focus_request();
        if restore && let Some(window) = &self.window {
            window.set_minimized(false);
        }
        let window = match target {
            Some(CoreWindowId::Main) => self.window.as_ref(),
            Some(CoreWindowId::Equalizer) => self.eq_window.as_ref(),
            Some(CoreWindowId::Playlist) => self.playlist_window.as_ref(),
            _ => None,
        };
        if let Some(window) = window {
            window.focus_window();
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
            let attributes = Self::window_attributes(
                PresenterWindow::Equalizer,
                "Rustamp Equalizer",
                (width, height),
                self.saved_position(CoreWindowId::Equalizer),
            );
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
        }
        let desired = self.app.eq_frame_size();
        if self.applied_eq_client != Some(desired)
            && let Some(window) = &self.eq_window
        {
            let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
            self.applied_eq_client = Some(desired);
        }
    }

    /// Applies only model positions; detached windows are never forced back
    /// into the default stack. Borderless outer/client origins coincide.
    fn apply_docking_positions(&mut self) {
        for (id, geometry, window) in [
            (CoreWindowId::Main, self.stack.main, self.window.as_ref()),
            (
                CoreWindowId::Equalizer,
                self.stack.equalizer,
                self.eq_window.as_ref(),
            ),
            (
                CoreWindowId::Playlist,
                self.stack.playlist,
                self.playlist_window.as_ref(),
            ),
        ] {
            let target = PhysicalPosition::new(geometry.x, geometry.y);
            if let Some(window) = window
                && window.is_minimized() != Some(true)
                && window.outer_position().ok() != Some(target)
            {
                window.set_outer_position(target);
            }
            self.app.set_window_position(id, geometry.x, geometry.y);
        }
    }

    /// Acknowledgements of our own placement do nothing. Ignore stale queued
    /// Moved events (the live position has already changed again), and Windows'
    /// minimized sentinel coordinates. Secondary moves snap to visible peers;
    /// moving the main translates its exact connected component in the model.
    fn classic_window_moved(&mut self, kind: WindowKind, position: PhysicalPosition<i32>) {
        let window = match kind {
            WindowKind::Main => self.window.as_ref(),
            WindowKind::Equalizer => self.eq_window.as_ref(),
            WindowKind::Playlist => self.playlist_window.as_ref(),
        };
        let Some(window) = window else {
            return;
        };
        if window.is_minimized() == Some(true) || window.outer_position().ok() != Some(position) {
            return;
        }
        let geometry = match kind {
            WindowKind::Main => self.stack.main,
            WindowKind::Equalizer => self.stack.equalizer,
            WindowKind::Playlist => self.stack.playlist,
        };
        if (geometry.x, geometry.y) == (position.x, position.y) {
            return;
        }
        if kind == WindowKind::Main {
            self.stack.move_main(
                position.x.saturating_sub(geometry.x),
                position.y.saturating_sub(geometry.y),
            );
        } else {
            let targets: Vec<ClassicWindow> = [
                (self.stack.main, self.app.main_visible()),
                (self.stack.equalizer, self.eq_window.is_some()),
                (self.stack.playlist, self.playlist_window.is_some()),
            ]
            .into_iter()
            .filter_map(|(target, visible)| (visible && target.kind != kind).then_some(target))
            .collect();
            let snapped = geometry.snapped_position(
                WindowPosition::new(position.x, position.y),
                &targets,
                self.app.config().ui.snap_distance,
            );
            let moved = match kind {
                WindowKind::Equalizer => &mut self.stack.equalizer,
                WindowKind::Playlist => &mut self.stack.playlist,
                WindowKind::Main => unreachable!(),
            };
            moved.x = snapped.x;
            moved.y = snapped.y;
        }
        self.apply_docking_positions();
    }

    /// Synchronizes current live OS window positions, visibility, and shade states
    /// into the app configuration so they are preserved on exit.
    pub(crate) fn sync_window_layouts_to_app(&mut self) {
        self.apply_docking_positions();
        self.app
            .set_window_shaded(CoreWindowId::Main, self.app.state().windowshade);
        self.app.set_window_visible(CoreWindowId::Main, true);

        let eq_visible = self.app.eq_open();
        self.app
            .set_window_visible(CoreWindowId::Equalizer, eq_visible);
        self.app
            .set_window_shaded(CoreWindowId::Equalizer, self.app.eq_state().windowshade);

        let pl_visible = self.app.playlist_open();
        self.app
            .set_window_visible(CoreWindowId::Playlist, pl_visible);
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

    /// Shows the main menu as the operating system's own popup and routes
    /// the pick.
    ///
    /// The menu used to be a window of ours, painted from the skin's bitmap
    /// font: it carried a standard window caption (it was an ordinary
    /// top-level window) and its text could not be read (a small fixed-cell
    /// glyph sheet in 35-px rows). It is now the system menu, which is also
    /// the faithful answer - S04 §1 settles the reference popup's window
    /// class as `#32768`, the class Windows uses for menus - so the
    /// separators, the submenu arrows, the right-aligned accelerators, the
    /// tick marks, the frameless popup shape and the system font all come
    /// from the OS ([`rustamp_platform::show_popup_menu`]).
    ///
    /// It is drained from the tick, never from a window procedure: the menu
    /// runs its own modal message loop, exactly like the tray popup.
    /// `TrackPopupMenu` returns when the user has chosen or dismissed, so the
    /// app's popup flag is cleared here either way and the player never holds
    /// a popup state with no popup on screen.
    ///
    /// The anchor is the measured menu-button point ([`menu_popup_origin`])
    /// in screen coordinates, which is now exact: the main window is
    /// borderless, so its outer position *is* its client origin.
    fn drain_main_menu(&mut self) {
        if !self.app.main_menu_is_open() {
            return;
        }
        self.app.close_main_menu();
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            let main_outer = self
                .window
                .as_ref()
                .and_then(|window| window.outer_position().ok())
                .map(|position| (position.x, position.y))
                .unwrap_or((DEFAULT_MAIN_POSITION.x, DEFAULT_MAIN_POSITION.y));
            let menu = self.app.native_main_menu();
            match rustamp_platform::show_popup_menu(
                owner,
                &menu.rows,
                menu_popup_origin(main_outer),
            ) {
                Ok(None) => {} // Dismissed: not a selection, nothing to do.
                Ok(Some(id)) => match menu.choice_for_id(id) {
                    Some(choice) => {
                        if let Err(error) = self.app.activate_menu_choice(choice) {
                            eprintln!("rustamp: menu item failed: {error}");
                        }
                    }
                    None => eprintln!("rustamp: the menu reported an unknown id {id}"),
                },
                Err(error) => eprintln!("rustamp: no main menu: {error}"),
            }
        }
        #[cfg(not(windows))]
        eprintln!("rustamp: a native popup menu is only implemented on Windows");
    }

    /// Shows a pending clutterbar popup (Options or visualization menu,
    /// RFC 0016) as the system's own menu, like [`Windowed::drain_main_menu`].
    ///
    /// Anchored at the letter's click point (S04 §1); the screen position is
    /// unmeasured, so that is our choice.
    fn drain_clutter_menu(&mut self) {
        let Some(kind) = self.app.take_clutter_menu() else {
            return;
        };
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            let main_outer = self
                .window
                .as_ref()
                .and_then(|window| window.outer_position().ok())
                .map(|position| (position.x, position.y))
                .unwrap_or((DEFAULT_MAIN_POSITION.x, DEFAULT_MAIN_POSITION.y));
            let point = match kind {
                crate::menu::ClutterMenu::Options => rustamp_ui::menu::CLUTTER_O_POINT,
                crate::menu::ClutterMenu::Visualization => rustamp_ui::menu::CLUTTER_V_POINT,
            };
            let menu = self.app.native_clutter_menu(kind);
            match rustamp_platform::show_popup_menu(
                owner,
                &menu.rows,
                rustamp_ui::menu::clutter_popup_origin(main_outer, point),
            ) {
                Ok(None) => {}
                Ok(Some(id)) => match menu.choice_for_id(id) {
                    Some(choice) => {
                        if let Err(error) = self.app.activate_menu_choice(choice) {
                            eprintln!("rustamp: menu item failed: {error}");
                        }
                    }
                    None => eprintln!("rustamp: the menu reported an unknown id {id}"),
                },
                Err(error) => eprintln!("rustamp: no {kind:?} menu: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = kind;
            eprintln!("rustamp: a native popup menu is only implemented on Windows");
        }
    }
    /// Shows a pending equalizer Presets popup (S07 §4.2: Load / Save /
    /// Delete) as the system's own menu, below the Presets button.
    ///
    /// The button sits at (217, 18, 44, 12) in the equalizer's layout
    /// (S07 §4.1), doubled with the double-size mode; the popup's screen
    /// position is unmeasured, so the button's bottom-left is our choice.
    fn drain_eq_menu(&mut self) {
        if !self.app.take_eq_menu() {
            return;
        }
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            let eq_outer = self
                .eq_window
                .as_ref()
                .and_then(|window| window.outer_position().ok())
                .map(|position| (position.x, position.y))
                .unwrap_or((DEFAULT_MAIN_POSITION.x, DEFAULT_MAIN_POSITION.y));
            let scale = if self.app.state().double_size { 2 } else { 1 };
            let menu = self.app.native_eq_menu();
            match rustamp_platform::show_popup_menu(
                owner,
                &menu.rows,
                (eq_outer.0 + 217 * scale, eq_outer.1 + 30 * scale),
            ) {
                Ok(None) => {}
                Ok(Some(id)) => match menu.choice_for_id(id) {
                    Some(crate::eq_presets::EqMenuChoice::LoadFromFile) => {
                        match rustamp_platform::pick_eqf_file(owner, "Load EQ preset") {
                            Ok(None) => {}
                            Ok(Some(path)) => match self.app.load_eq_file(&path) {
                                Ok(name) => println!("rustamp: loaded EQ preset {name}"),
                                Err(error) => eprintln!("rustamp: EQ preset not loaded: {error}"),
                            },
                            Err(error) => eprintln!("rustamp: no file dialog: {error}"),
                        }
                        self.request_redraw_eq();
                    }
                    Some(choice) => {
                        if let Err(error) = self.app.activate_eq_menu_choice(choice) {
                            eprintln!("rustamp: presets menu item failed: {error}");
                        }
                        self.request_redraw_eq();
                    }
                    None => eprintln!("rustamp: the presets menu reported an unknown id {id}"),
                },
                Err(error) => eprintln!("rustamp: no presets menu: {error}"),
            }
        }
        #[cfg(not(windows))]
        eprintln!("rustamp: a native popup menu is only implemented on Windows");
    }

    /// Shows a pending standard file-open dialog and adds what was chosen.
    ///
    /// The ADD button and the open-media actions ask for files. They used to
    /// open a window of ours that wanted a typed path or a drop onto it;
    /// Windows has a file-open dialog for exactly this, with multiple
    /// selection, so the player uses it
    /// ([`rustamp_platform::pick_media_files`]).
    ///
    /// Where the seam sits, and why here: the headless half keeps its model -
    /// a command still *requests* a file choice, which is what the app's
    /// tests pin - and the presenter decides how the OS asks. The dialog is
    /// modal and runs its own message loop, so it is opened from the tick,
    /// never from a window procedure (the rule the tray popup follows).
    ///
    /// Multiple selection is always allowed: both the playlist's ADD (S06 §4)
    /// and the main window's open-media actions add a list. The chosen files
    /// become one [`Command::AddItems`] carrying the requesting command's
    /// `play` flag untouched, so `Play file` still starts and the playlist's
    /// ADD still only appends. A cancelled dialog adds nothing.
    ///
    /// The other two faces keep their own windows: a directory pick and a
    /// location/URL entry are not file-open dialogs (the directory picker
    /// would be a different native dialog, which no spec covers yet).
    fn drain_media_pick(&mut self) {
        let Some((kind, play)) = self.pending_media_pick.take() else {
            return;
        };
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            match rustamp_platform::pick_media_files(owner, kind.title(), true) {
                Ok(paths) if paths.is_empty() => {} // Cancelled: nothing to add.
                Ok(paths) => {
                    let items = rustamp_ui::media_items_for_paths(&paths);
                    let command = Command::AddItems {
                        items,
                        at: None,
                        play,
                    };
                    if let Err(error) = self.app.submit(command) {
                        eprintln!("rustamp: the chosen files could not be added: {error}");
                    }
                    self.app.sync_playlist_entries();
                }
                Err(error) => eprintln!("rustamp: no file dialog: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (kind, play);
            eprintln!("rustamp: a native file dialog is only implemented on Windows");
        }
    }

    /// Shows the native save-playlist dialog when Ctrl+S was pressed.
    ///
    /// Ctrl+S in the playlist window (command ID 40204, S03 §2) requests a
    /// save. The presenter opens the native save dialog (modal, synchronous)
    /// and sends [`Command::SavePlaylist`] with the chosen path. A cancelled
    /// dialog saves nothing.
    fn drain_save_playlist(&mut self) {
        if !std::mem::take(&mut self.app.request_save_playlist) {
            return;
        }
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            let default_name = format!(
                "playlist.{}",
                self.app
                    .loaded_playlist_format()
                    .unwrap_or(rustamp_playlist::PlaylistFormat::M3u)
            );
            match rustamp_platform::save_playlist_file(owner, "Save Playlist", &default_name) {
                Ok(Some(path)) => {
                    let command = Command::SavePlaylist(path.clone());
                    if let Err(error) = self.app.submit(command) {
                        eprintln!("rustamp: save playlist failed: {error}");
                    } else {
                        println!("rustamp: playlist saved to {}", path.display());
                    }
                }
                Ok(None) => {} // User cancelled
                Err(error) => eprintln!("rustamp: no save dialog: {error}"),
            }
        }
        #[cfg(not(windows))]
        eprintln!("rustamp: a native save dialog is only implemented on Windows");
    }

    /// Opens (or raises) the native preferences dialog, then relays what the
    /// user did in it. The dialog is modeless: it lives on this thread's
    /// message loop, so the tick only has to feed it the player's settings and
    /// take its actions back.
    fn drain_prefs(&mut self) {
        #[cfg(windows)]
        {
            use rustamp_platform::PrefsWindow;

            if let Some(page) = self.pending_prefs.take() {
                let node = crate::dialogs::prefs_node_for(page);
                match self.prefs_window {
                    Some(window) if window.is_open() => {
                        window.show_page(node);
                        window.raise();
                    }
                    _ => {
                        // Fresh folder and approval list each time it opens.
                        self.app.rescan_plugins();
                        let settings = self.app.prefs_settings();
                        match PrefsWindow::open(self.raw_main_window(), node, &settings) {
                            Ok(window) => self.prefs_window = Some(window),
                            Err(error) => eprintln!("rustamp: no preferences dialog: {error}"),
                        }
                    }
                }
            }
            let Some(window) = self.prefs_window else {
                return;
            };
            if !window.is_open() {
                self.prefs_window = None;
                return;
            }
            for action in window.take_actions() {
                self.app.apply_prefs_action(action);
            }
            window.set_settings(&self.app.prefs_settings());
        }
        #[cfg(not(windows))]
        {
            if self.pending_prefs.take().is_some() {
                eprintln!("rustamp: a native preferences dialog is only implemented on Windows");
            }
        }
    }

    /// The main window's raw OS handle, or `0` when it has none yet.
    ///
    /// The one reader for the places that need the window itself rather than
    /// `winit`'s wrapper: the tray icon attach and the native dialogs.
    #[cfg(windows)]
    fn raw_main_window(&self) -> isize {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Some(window) = &self.window else {
            return 0;
        };
        match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            _ => 0,
        }
    }

    /// Paints every open window with this tick's frame.
    ///
    /// The animated parts of the main window - the visualization (the "VU
    /// meter"), the marquee and the time display - are driven by the clock in
    /// [`App::pump`] / [`App::render`], which this tick already ran. What was
    /// missing is the second half: the frame has to reach the screen on the
    /// same tick.
    ///
    /// This used to ask the OS for a repaint (`request_redraw`, i.e.
    /// `RedrawWindow` -> `WM_PAINT` -> `RedrawRequested` -> `paint`), so a
    /// frame was only presented if and when that window message came back.
    /// The animation then lives or dies by the delivery of a window message,
    /// and the owner's machine showed it dying: the visualization moved only
    /// while the main window had focus. There is nothing to gain from the
    /// detour - the frames are drawn into a `softbuffer` surface we own and
    /// presented with an ordinary blit, which needs no `WM_PAINT` - so the
    /// tick paints directly and the animation runs whether or not the window
    /// is focused or in the foreground.
    ///
    /// `RedrawRequested` is still handled for repaints the OS itself asks for
    /// (an uncovered or resized window), and the input paths still request
    /// one so a press is reflected without waiting for the next tick.
    fn present_frames(&mut self) {
        self.paint();
        self.paint_eq();
        self.paint_dialog();
        self.paint_playlist();
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
        // A native dialog is not a window of ours: see
        // [`Windowed::drain_media_pick`]. The request is only recorded here,
        // because this runs from window procedures too.
        if let DialogRoute::NativeFilePicker { play } = DialogRoute::for_dialog(kind) {
            self.app.close_dialog(kind);
            self.pending_media_pick = Some((kind, play));
            return;
        }
        if let DialogRoute::NativePreferences { page } = DialogRoute::for_dialog(kind) {
            self.app.close_dialog(kind);
            self.pending_prefs = Some(page);
            return;
        }
        let (width, height) = kind.client_size();
        let attributes =
            Self::window_attributes(PresenterWindow::Dialog, kind.title(), (width, height), None);
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
    /// The dialog consumes `Enter` (confirm the jump box, the skin browser,
    /// the open-media dialog, or the load-playlist dialog, which loads the
    /// chosen playlist file; the preferences dialog is a no-op, its contents
    /// unmeasured), `Escape` (close), `Backspace`, typed text (the open-media
    /// and load-playlist fields and the jump box, see [`App::dialog_text`])
    /// and the arrows of the skin browser; the window closes itself when the
    /// app reports no open dialog afterwards.
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

    /// Attributes for one of the player's OS windows: the shared skinned
    /// shell for `window` plus the saved position, when the config has one.
    ///
    /// The shell itself lives in [`rustamp_ui::shell::skinned_window_attributes`]
    /// — borderless (the title bar is part of the skin, S01 §1) and
    /// non-resizable, with the **client** area named in physical pixels. This
    /// crate therefore never calls `Window::default_attributes()` itself; a
    /// window that did would be decorated, which is the defect the shell
    /// exists for. `size` is the client size the caller already computed
    /// (`App::frame_size`, `App::eq_frame_size`, a spec'd constant), so the
    /// geometry the tests measure is passed through untouched.
    fn window_attributes(
        window: PresenterWindow,
        title: &str,
        size: (u32, u32),
        position: Option<(i32, i32)>,
    ) -> WindowAttributes {
        let attributes =
            rustamp_ui::shell::skinned_window_attributes(title, size, window.taskbar_role());
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows as _;
            if window == PresenterWindow::Main {
                attributes.with_class_name("Winamp v1.x")
            } else {
                attributes
            }
        };
        match position {
            Some((x, y)) => attributes.with_position(PhysicalPosition::new(x, y)),
            None => attributes,
        }
    }

    /// Creation uses the restored/live model, including retained closed windows.
    fn saved_position(&self, id: CoreWindowId) -> Option<(i32, i32)> {
        let window = match id {
            CoreWindowId::Main => self.stack.main,
            CoreWindowId::Equalizer => self.stack.equalizer,
            CoreWindowId::Playlist => self.stack.playlist,
            _ => return None,
        };
        Some((window.x, window.y))
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
        let attributes = Self::window_attributes(
            PresenterWindow::Playlist,
            "Rustamp Playlist",
            (width, height),
            self.saved_position(CoreWindowId::Playlist),
        );
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
        self.apply_docking_positions();
        self.request_playlist_redraw();
    }

    /// Events for the playlist editor window, in playlist-window coordinates.
    ///
    /// Files dropped from the shell are added ([`App::drop_on_playlist`]).
    /// Pointer input routes to the headless row-drag model; leave and focus
    /// loss cancel it. Playlist keys use the playlist-specific shortcut path.
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
                self.classic_window_moved(WindowKind::Playlist, position);
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
            WindowEvent::DroppedFile(path) => self.drop_file(path),
            WindowEvent::Focused(focused) => {
                // Per-window activation is open, so the playlist shares the
                // main focus flag (see `App::render_playlist`).
                if !focused {
                    self.app.playlist_mouse_leave();
                    self.last_playlist_click = None;
                }
                self.request_playlist_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => self.playlist_key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                // No double-size scale: the playlist ignores it (S05 §2.1).
                self.playlist_cursor = (position.x as i32, position.y as i32);
                self.app
                    .playlist_mouse_move(self.playlist_cursor.0, self.playlist_cursor.1);
                if self.app.playlist().drag_insertion().is_some() {
                    self.last_playlist_click = None;
                }
                self.request_playlist_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.app.playlist_mouse_leave();
                self.last_playlist_click = None;
                self.request_playlist_redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.playlist_cursor;
                if state == ElementState::Pressed
                    && (0..rustamp_ui::playlist_window::TITLE_BAR_HEIGHT).contains(&y)
                    && rustamp_ui::playlist_window::hit_test_title_bar(x, y).is_none()
                    && let Some(window) = &self.playlist_window
                    && window.drag_window().is_ok()
                {
                    self.last_playlist_click = None;
                    return;
                }
                // Row dragging is an own choice while S06 §8 Q6 stays open.
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
                self.apply_docking_positions();
                self.request_playlist_redraw();
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// A file dropped from the shell onto the main, equalizer or playlist
    /// window: appended to the playlist the way the ADD button does
    /// ([`App::drop_on_playlist`]), whichever window received it. `winit`
    /// delivers one event per file, so a multi-file drop arrives here several
    /// times and keeps the shell's order.
    fn drop_file(&mut self, path: PathBuf) {
        match self.app.drop_on_playlist(std::slice::from_ref(&path)) {
            Ok(_) => {
                self.request_playlist_redraw();
                self.request_redraw();
                self.request_redraw_eq();
            }
            Err(error) => eprintln!("rustamp: dropped file not added: {error}"),
        }
    }

    /// Starts an OS window drag for a background press when easy-move is on
    /// ([`App::easy_drag_main`]). Returns whether the press was consumed by
    /// the drag, so the control path is skipped; a failed drag falls through
    /// to the controls so the press is never swallowed. Only the VM can prove
    /// it (no window may open here); the decision itself is headless-tested
    /// on [`App::easy_drag_main`].
    fn try_easy_drag(&mut self, x: i32, y: i32) -> Option<()> {
        let title = rustamp_ui::hit_test(&rustamp_ui::MainWindowLayout::CLASSIC, x, y)
            == Some(rustamp_ui::Widget::TitleBar);
        if !title && !self.app.easy_drag_main(x, y) {
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
        let owner = self.raw_main_window();
        if owner == 0 {
            eprintln!("rustamp: tray icon unavailable: the main window has no Win32 handle");
            return;
        }
        match rustamp_platform::TrayIcon::attach(owner) {
            Ok(tray) => self.tray = Some(tray),
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
        let attributes = Self::window_attributes(
            PresenterWindow::Main,
            "Rustamp",
            (width, height),
            self.saved_position(CoreWindowId::Main),
        );
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
        #[cfg(windows)]
        if let Some(window) = &self.window {
            match crate::external_control::native::ExternalControl::attach(
                Arc::clone(window),
                self.app.host_player_state(),
            ) {
                Ok(control) => self.external_control = Some(control),
                Err(error) => eprintln!("rustamp: external control unavailable: {error}"),
            }
        }
        self.sync_window_state();
        self.sync_eq_window(event_loop);
        self.request_redraw();
        self.request_redraw_eq();
        self.sync_dialog_window(event_loop);
        self.request_dialog_redraw();
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
                self.classic_window_moved(WindowKind::Main, position);
            }
            WindowEvent::DroppedFile(path) => self.drop_file(path),
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
                self.sync_window_state();
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
                // The menu dismisses itself: a system popup owns the input
                // while it is up (`TrackPopupMenu`), so there is no stale
                // popup left for a press here to close.
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
                let (x, y) = self.cursor;
                if let Err(error) = self.app.mouse_right_down(x, y) {
                    eprintln!("rustamp: right-click failed: {error}");
                }
                self.sync_eq_window(event_loop);
                self.sync_playlist_window(event_loop);
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(windows)]
        if let Some(control) = &self.external_control {
            control.drain(&mut self.app);
        }
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
        #[cfg(windows)]
        if let Some(control) = &self.external_control {
            control.publish(self.app.host_player_state());
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
        self.apply_docking_positions();
        // Dialogs follow the app's open dialog (opened by menu or shortcut,
        // closed by Escape, Enter or the window's close button).
        self.sync_dialog_window(event_loop);
        // The playlist editor follows its open flag and, while open, tracks
        // the main window (moves and shade reflow).
        self.sync_playlist_window(event_loop);
        self.apply_docking_positions();
        // The frame of this tick, painted here rather than asked for: see
        // [`Windowed::present_frames`].
        self.present_frames();
        // Outside every window procedure, like the tray popup: both of
        // these run their own modal message loop.
        self.drain_main_menu();
        self.drain_clutter_menu();
        self.drain_eq_menu();
        self.drain_media_pick();
        self.drain_save_playlist();
        self.drain_prefs();
        // After every window has been synced, so a focus request for a window
        // that opened this frame finds its OS window.
        self.apply_focus_requests();
        // An error the user must see goes to the native title (BETA-04).
        self.sync_title();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Remove the icon/callback before winit tears down the owning HWND.
        #[cfg(windows)]
        {
            self.external_control = None;
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
                self.classic_window_moved(WindowKind::Equalizer, position);
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
            WindowEvent::DroppedFile(path) => self.drop_file(path),
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
                if state == ElementState::Pressed
                    && rustamp_ui::equalizer::hit_test(&rustamp_ui::EqualizerLayout::CLASSIC, x, y)
                        == Some(rustamp_ui::EqWidget::TitleBar)
                    && let Some(window) = &self.eq_window
                    && window.drag_window().is_ok()
                {
                    self.eq_left_down = false;
                    return;
                }
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

    #[test]
    fn double_size_is_a_first_start_default_on_high_dpi_only() {
        // First start on a 150 % / 200 % display: double size.
        assert!(first_start_double_size(false, Some(144)));
        assert!(first_start_double_size(false, Some(192)));
        // A 100 % / 125 % display, or an unknown one, keeps the classic size.
        assert!(!first_start_double_size(false, Some(96)));
        assert!(!first_start_double_size(false, Some(120)));
        assert!(!first_start_double_size(false, None));
        // Once a config exists the saved choice rules, whatever the DPI.
        assert!(!first_start_double_size(true, Some(192)));
    }

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

    #[test]
    fn saved_geometry_restores_connections_without_forcing_detached_windows() {
        let mut presenter = desktop_test_presenter(false);
        presenter.stack.move_main(-100, 80);
        presenter.stack.equalizer.x = 700;
        presenter.stack.equalizer.y = 400;
        presenter
            .app
            .submit(Command::ToggleWindowShade(CoreWindowId::Main))
            .unwrap();
        presenter.sync_window_state();
        presenter.sync_window_layouts_to_app();
        let saved = crate::config::serialize_honest(&presenter.app.current_config()).unwrap();
        let config = crate::config::parse(&saved).unwrap();
        let restored_app = App::new(AppOptions {
            config,
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: None,
            seconds: None,
        })
        .unwrap();
        let mut restored = Windowed::new(restored_app);
        assert_eq!(restored.stack, presenter.stack);
        restored.stack.move_main(30, -40);
        assert_eq!(
            (restored.stack.equalizer.x, restored.stack.equalizer.y),
            (700, 400)
        );
        assert!(
            restored.window.is_none(),
            "configuration test must stay headless"
        );
    }

    /// Exercises actual Moved events and OS readback through the production
    /// presenter, not a lookalike Probe window. No input is synthesized: this
    /// pins Rustamp's explicit programmatic-move snap policy. Real pointer
    /// tracking during winit's modal drag still needs interactive VM acceptance.
    #[test]
    #[ignore = "VM-only: opens the real three-window presenter; run in the guest interactive session"]
    fn docking_presenter_moves_snaps_detaches_reflows_and_restores() {
        require_test_vm();
        struct Probe {
            presenter: Windowed,
            phase: u8,
            next: Instant,
            deadline: Instant,
        }
        impl Probe {
            fn assert_positions(&self, expected: [(i32, i32); 3]) {
                for (window, expected) in [
                    self.presenter.window.as_ref(),
                    self.presenter.eq_window.as_ref(),
                    self.presenter.playlist_window.as_ref(),
                ]
                .into_iter()
                .zip(expected)
                {
                    let position = window.unwrap().outer_position().unwrap();
                    assert_eq!((position.x, position.y), expected, "phase {}", self.phase);
                }
            }
        }
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.presenter.resumed(event_loop);
            }
            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                id: WindowId,
                event: WindowEvent,
            ) {
                self.presenter.window_event(event_loop, id, event);
            }
            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                assert!(Instant::now() < self.deadline, "docking probe timed out");
                self.presenter.about_to_wait(event_loop);
                if Instant::now() < self.next {
                    return;
                }
                match self.phase {
                    0 => {
                        self.assert_positions([(26, 29), (26, 145), (26, 261)]);
                        self.presenter
                            .window
                            .as_ref()
                            .unwrap()
                            .set_outer_position(PhysicalPosition::new(126, 29));
                    }
                    1 => {
                        self.assert_positions([(126, 29), (126, 145), (126, 261)]);
                        self.presenter
                            .eq_window
                            .as_ref()
                            .unwrap()
                            .set_outer_position(PhysicalPosition::new(600, 400));
                    }
                    2 => {
                        self.assert_positions([(126, 29), (600, 400), (126, 261)]);
                        self.presenter
                            .playlist_window
                            .as_ref()
                            .unwrap()
                            .set_outer_position(PhysicalPosition::new(132, 150));
                    }
                    3 => {
                        self.assert_positions([(126, 29), (600, 400), (126, 145)]);
                        self.presenter
                            .window
                            .as_ref()
                            .unwrap()
                            .set_outer_position(PhysicalPosition::new(226, 129));
                    }
                    4 => {
                        self.assert_positions([(226, 129), (600, 400), (226, 245)]);
                        self.presenter
                            .app
                            .submit(Command::ToggleWindowShade(CoreWindowId::Main))
                            .unwrap();
                    }
                    5 => {
                        self.assert_positions([(226, 129), (600, 400), (226, 143)]);
                        self.presenter.sync_window_layouts_to_app();
                        let text =
                            crate::config::serialize_honest(&self.presenter.app.current_config())
                                .unwrap();
                        let app = App::new(AppOptions {
                            config: crate::config::parse(&text).unwrap(),
                            skin: crate::skin::builtin(),
                            output: OutputChoice::Null,
                            volume: 200,
                            file: None,
                            seconds: None,
                        })
                        .unwrap();
                        self.presenter = Windowed::new(app);
                        self.presenter.resumed(event_loop);
                    }
                    6 => {
                        self.assert_positions([(226, 129), (600, 400), (226, 143)]);
                        self.presenter
                            .window
                            .as_ref()
                            .unwrap()
                            .set_outer_position(PhysicalPosition::new(326, 129));
                    }
                    7 => {
                        self.assert_positions([(326, 129), (600, 400), (326, 143)]);
                        event_loop.exit();
                        return;
                    }
                    _ => unreachable!(),
                }
                self.phase += 1;
                self.next = Instant::now() + Duration::from_millis(150);
            }
        }
        let mut presenter = desktop_test_presenter(false);
        presenter
            .app
            .submit(Command::ToggleWindow(CoreWindowId::Equalizer))
            .unwrap();
        let mut probe = Probe {
            presenter,
            phase: 0,
            next: Instant::now() + Duration::from_millis(150),
            deadline: Instant::now() + Duration::from_secs(10),
        };
        build_event_loop(true).unwrap().run_app(&mut probe).unwrap();
        assert_eq!(probe.phase, 7);
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

    /// The presenter half that is decidable without a desktop: it takes both
    /// requests every call, whether or not an OS window exists to receive
    /// them, so nothing stays queued for a later frame.
    #[test]
    fn the_presenter_consumes_focus_and_restore_requests_without_leaking_them() {
        let mut presenter = desktop_test_presenter(false);
        presenter.app.submit(Command::FocusNextWindow).unwrap();
        presenter.app.submit(Command::ShowMainWindow).unwrap();
        presenter.apply_focus_requests();
        assert_eq!(presenter.app.take_focus_request(), None);
        assert!(!presenter.app.take_restore_request());
        // A second call with nothing pending is a no-op.
        presenter.apply_focus_requests();
        assert_eq!(presenter.app.take_focus_request(), None);
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

    /// The frame defect, pinned at the decision rather than at the pixels:
    /// every window the presenter opens asks for a **borderless** shell with
    /// the client size its spec measures.
    ///
    /// Why it has to be the attributes: a native caption does not change the
    /// client area, so `inner_size()` is the same with and without a frame
    /// and no geometry test can tell them apart. This fails if a window is
    /// created decorated, and it fails if the frame change moved the client
    /// size that S01/S04/S05 pin.
    #[test]
    fn every_window_the_presenter_opens_is_borderless_at_its_measured_client_size() {
        let presenter = desktop_test_presenter(false);
        let dialog = DialogKind::About;
        let windows: [(PresenterWindow, &str, (u32, u32)); 4] = [
            (PresenterWindow::Main, "Rustamp", presenter.app.frame_size()),
            (
                PresenterWindow::Equalizer,
                "Rustamp Equalizer",
                presenter.app.eq_frame_size(),
            ),
            (
                PresenterWindow::Playlist,
                "Rustamp Playlist",
                PLAYLIST_WINDOW_SIZE,
            ),
            (
                PresenterWindow::Dialog,
                dialog.title(),
                dialog.client_size(),
            ),
        ];
        for (window, title, size) in windows {
            let attributes = Windowed::window_attributes(window, title, size, None);
            assert!(
                !attributes.decorations,
                "{title}: the title bar is part of the skin, so the window carries no native frame"
            );
            assert!(!attributes.resizable, "{title}: fixed size");
            assert_eq!(
                rustamp_ui::attributes_client_size(&attributes),
                Some(size),
                "{title}: the client area must pass through the shell untouched"
            );
        }
        // The two measured sizes themselves, so a drifted constant is caught
        // here and not only in the VM.
        assert_eq!(presenter.app.frame_size(), (275, 116));
        assert_eq!(presenter.app.eq_frame_size(), (275, 116));
        assert_eq!(PLAYLIST_WINDOW_SIZE, (275, 116));
    }

    /// The ADD defect: the button opened a window of ours that asked for a
    /// typed path or a drop onto it, instead of the standard Windows
    /// file-open dialog.
    ///
    /// The routing decision is checked here: the files face is native and
    /// carries the requesting command's `play` flag, and no other dialog
    /// changed. Multiple selection and the decoding of what the user picked
    /// are checked in `rustamp-platform` (`file_dialog::parse_selection`).
    /// That the OS then shows the dialog needs a desktop, so only the owner's
    /// machine can confirm it.
    #[test]
    fn only_the_files_face_of_open_media_is_the_native_file_dialog() {
        for play in [true, false] {
            assert_eq!(
                DialogRoute::for_dialog(DialogKind::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play,
                }),
                DialogRoute::NativeFilePicker { play },
                "the files face is the standard file-open dialog, with the play flag kept"
            );
        }
        for kind in [
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            },
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            },
            DialogKind::About,
            DialogKind::FileInfo,
            DialogKind::LoadPlaylist,
            DialogKind::SkinBrowser,
        ] {
            assert_eq!(
                DialogRoute::for_dialog(kind),
                DialogRoute::OwnWindow,
                "{kind:?} has no OS dialog and keeps its own window"
            );
        }
        for page in [PrefsPage::VisPlugin, PrefsPage::VisOptions] {
            assert_eq!(
                DialogRoute::for_dialog(DialogKind::Preferences(page)),
                DialogRoute::NativePreferences { page },
                "preferences is a real Windows dialog"
            );
        }
    }

    /// The pick is drained from the tick, so one request shows one dialog and
    /// a drained request is gone - a modal dialog opened twice, or opened
    /// from inside a window procedure, is how this path breaks.
    #[test]
    fn a_pending_file_dialog_is_drained_exactly_once() {
        let mut presenter = desktop_test_presenter(false);
        presenter.pending_media_pick = Some((
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            },
            false,
        ));
        // No main window, so there is no owner handle and the platform call
        // refuses before any dialog exists; the request is still consumed.
        presenter.drain_media_pick();
        assert!(presenter.pending_media_pick.is_none());
        presenter.drain_media_pick();
        assert!(presenter.pending_media_pick.is_none());
        assert!(presenter.window.is_none(), "the test must stay headless");
    }

    /// The animation defect: the main window's visualization (the "VU meter")
    /// moved only while the window had focus.
    ///
    /// What a headless test can pin is that focus is not an input to the
    /// animated state: the envelopes advance from the clock alone, with the
    /// window explicitly unfocused. This fails if anyone makes the animation
    /// depend on focus or on activation.
    ///
    /// What it cannot pin is the other half of the defect - that the frame
    /// reaches the screen on every tick. That needs a window on a desktop;
    /// the tick now paints directly instead of asking the OS for a repaint
    /// (see [`Windowed::present_frames`]), and only the owner's machine can
    /// confirm the result.
    #[test]
    fn the_visualization_advances_with_the_clock_while_the_window_is_unfocused() {
        use rustamp_core::event::Event;
        use rustamp_core::vis::{VIS_SAMPLES, VisFrame};

        let mut presenter = desktop_test_presenter(false);
        presenter.app.set_focused(false);
        assert!(!presenter.app.focused, "the test must run unfocused");
        let frame = VisFrame {
            timestamp_us: 1,
            waveform: [std::array::from_fn(|n| {
                (std::f64::consts::TAU * 16.0 * n as f64 / VIS_SAMPLES as f64).sin() as f32
            }); 2],
            ..VisFrame::default()
        };
        presenter
            .app
            .ui_mut()
            .handle_event(&Event::StateChanged(TransportState::Playing), 0);
        presenter
            .app
            .ui_mut()
            .push_vis_frame(&frame, 32_000, 0)
            .expect("the analyser accepts a finite frame");
        let raised = presenter.app.state().visualization.bars;
        assert!(
            raised.iter().any(|row| *row > 0),
            "the pushed frame must raise at least one bar: {raised:?}"
        );
        // Time passes with no new frame and no focus change at all.
        presenter.app.ui_mut().refresh(400);
        let fallen = presenter.app.state().visualization.bars;
        assert_ne!(
            raised, fallen,
            "the envelopes must age from the clock, not from focus or a window message"
        );
        assert!(
            fallen
                .iter()
                .zip(raised.iter())
                .all(|(now, before)| now <= before),
            "they fall: {raised:?} -> {fallen:?}"
        );
        assert!(!presenter.app.focused, "focus was never touched");
        assert!(presenter.window.is_none(), "the test must stay headless");
    }

    /// The taskbar defect: the shell lists top-level windows, so the player
    /// appeared once per open window. Exactly one of its windows stands for
    /// the application, and it is the main window.
    ///
    /// What this can and cannot do: the decision is checked here, headlessly.
    /// That the shell then shows one button needs a taskbar, so it needs a
    /// desktop, and only the owner machine can confirm it.
    #[test]
    fn exactly_one_window_carries_the_players_taskbar_entry() {
        let application: Vec<PresenterWindow> = PresenterWindow::ALL
            .into_iter()
            .filter(|window| window.taskbar_role() == TaskbarRole::Application)
            .collect();
        assert_eq!(
            application,
            vec![PresenterWindow::Main],
            "the player is one application: only the main window may carry the taskbar entry"
        );
    }

    /// A restored position is applied at creation and does not bring the
    /// frame back (the position is the *outer* position, which is now the
    /// client position, because there is no frame between them).
    #[test]
    fn a_saved_position_is_applied_without_bringing_the_frame_back() {
        let attributes = Windowed::window_attributes(
            PresenterWindow::Main,
            "Rustamp",
            (275, 116),
            Some((26, 29)),
        );
        assert!(!attributes.decorations);
        assert_eq!(
            attributes
                .position
                .map(|position| position.to_physical::<i32>(1.0))
                .map(|position| (position.x, position.y)),
            Some((26, 29))
        );
        // Without a saved position the OS places the window.
        assert!(
            Windowed::window_attributes(PresenterWindow::Main, "Rustamp", (275, 116), None)
                .position
                .is_none()
        );
    }
}
