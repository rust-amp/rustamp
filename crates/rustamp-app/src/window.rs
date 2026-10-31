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
//! The saved positions are restored onto the screens that exist **now**
//! ([`rustamp_ui::placement`]): a window whose title strip - the only part of
//! a borderless window the user can drag - lies on no work area is pulled
//! back until it does. See `Windowed::restore_stack` for the rule.
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
use rustamp_platform::{
    DisplaySnapshot, HotkeyAction, init_dpi_awareness, raise_classic_group, single_instance,
    take_pending_hotkeys,
};
use rustamp_skin::CursorSlot;
use rustamp_ui::shell::TaskbarRole;
use rustamp_ui::{
    DEFAULT_MAIN_POSITION, DialogKind, Key, LIBRARY_LINE_HEIGHT, Modifiers, ShortcutOutcome,
    clamp_stack_to_screen,
    docking::{ClassicWindow, DockedStack, WindowKind, WindowPosition},
    menu::menu_popup_origin,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use crate::app::{App, WindowDirty};
use crate::clipboard::Clipboard;
use crate::cursor::{CursorAction, SkinCursors, cursor_action};
use crate::double_click::DoubleClick;

/// The equalizer window's class (S01 §1, ABI per S09 §1).
const EQUALIZER_CLASS: &str = "Winamp EQ";

/// The playlist editor's class (S01 §1, ABI per S09 §1).
const PLAYLIST_CLASS: &str = "Winamp PE";

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

/// The OS window class of each window, when it has an ABI one.
///
/// S09 §1 lists the classes plugins and external tools find the player by, and
/// S01 §1 names them for the three classic windows. The main window's name is
/// the single-instance class of S13 §2, so it comes from one place. Dialogs are
/// the player's own windows and carry none of these names.
fn window_class(window: PresenterWindow) -> Option<&'static str> {
    match window {
        PresenterWindow::Main => Some(rustamp_platform::single_instance::DEFAULT_CLASS),
        PresenterWindow::Equalizer => Some(EQUALIZER_CLASS),
        PresenterWindow::Playlist => Some(PLAYLIST_CLASS),
        PresenterWindow::Dialog => None,
    }
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

    /// Whether opening this window takes the keyboard focus from the window
    /// that has it.
    ///
    /// The main window does: it is the window the player starts in, and a
    /// dialog does: it is modal. The equalizer and the playlist editor do
    /// **not**: Winamp opens them without activating them, so the main window
    /// keeps the focus and keeps its focused title bar.
    ///
    /// The measured reference start state of the base-2.91 campaign is
    /// exactly that - "main focused, equalizer and playlist open" (the
    /// `base291-main-active` job, 2026-10-20). Our player drew the
    /// *unfocused* title bar there, because creating the equalizer's window
    /// activated it and took the focus away from the main window
    /// (2026-10-21). A real click on the equalizer's title bar still moves
    /// the focus - that is the `base291-main-inactive` job's real-mouse step,
    /// and it must keep working.
    const fn activates_on_open(self) -> bool {
        match self {
            PresenterWindow::Main | PresenterWindow::Dialog => true,
            PresenterWindow::Equalizer | PresenterWindow::Playlist => false,
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
    /// allowed.
    ///
    /// The requesting command's `play` flag is not part of the route: it
    /// rides along in the [`DialogKind`] the request came in as, and
    /// reaches the [`Command::AddItems`] the choice produces.
    NativeFilePicker,
    /// The shell's folder dialog: the **directory** face of the open-media
    /// dialog (S03 id 40187).
    ///
    /// A folder is not a file, so the file-open dialog cannot ask for one;
    /// the shell has its own dialog for it.
    NativeDirectoryPicker,
    /// The standard file-open dialog, for one playlist file.
    NativePlaylistPicker,
    /// The preferences dialog: a real Windows dialog (tree of pages, group
    /// boxes, check boxes) opened on `page`.
    NativePreferences { page: PrefsPage },
    /// A window of ours, painted from the dialog model.
    OwnWindow,
}

impl DialogRoute {
    /// Which route `kind` takes.
    ///
    /// The **files** and the **directory** faces of the open-media dialog
    /// are native: Windows has a file-open dialog for exactly the first,
    /// and the shell's folder dialog for exactly the second. Asking the
    /// user to type a path into a window of ours instead is the defect.
    /// A location/URL entry is not a file-open dialog, and everything else
    /// (About, file info, jump, the skin browser)
    /// is the player's own face with no OS equivalent. Load playlist is a
    /// native file dialog as well (S06 §4 measured a `#32770`).
    fn for_dialog(kind: DialogKind) -> Self {
        match kind {
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                ..
            } => DialogRoute::NativeFilePicker,
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Directory,
                ..
            } => DialogRoute::NativeDirectoryPicker,
            DialogKind::LoadPlaylist => DialogRoute::NativePlaylistPicker,
            DialogKind::Preferences(page) => DialogRoute::NativePreferences { page },
            _ => DialogRoute::OwnWindow,
        }
    }
}

/// The answer of a native media dialog.
///
/// The two native faces answer differently - the file dialog answers with a
/// list, the folder dialog with one path - so the answers are kept apart
/// here, and the *meaning* of each answer (cancelled, chosen, failed) is
/// the same for both.
#[derive(Debug)]
enum MediaPick {
    /// The files the user chose. An empty list is a cancelled dialog.
    Files(Vec<PathBuf>),
    /// The directory the user chose. `None` is a cancelled dialog.
    Directory(Option<PathBuf>),
    /// The file dialog failed.
    FileFailed(rustamp_platform::FileDialogError),
    /// The folder dialog failed.
    DirectoryFailed(rustamp_platform::FileDialogError),
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
    // The screens are read once, at the start: the first-start scale needs the
    // primary monitor's DPI, and the restore needs to know which screens the
    // saved window positions can point at.
    let snapshot = DisplaySnapshot::capture().ok();
    apply_first_start_scale(&mut app, snapshot.as_ref());
    let mut presenter = Windowed::new(app, snapshot.as_ref());
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
        presenter.app.save_session_playlist();
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
///
/// The display set is the one the start captured: without it the DPI is
/// unknown and the first start keeps the classic size.
fn apply_first_start_scale(app: &mut App, snapshot: Option<&DisplaySnapshot>) {
    let Some(path) = app.config_path() else {
        return;
    };
    let exists = path.exists();
    let dpi = snapshot.and_then(|snapshot| snapshot.primary().map(|monitor| monitor.dpi));
    if first_start_double_size(exists, dpi)
        && !app.state().double_size
        && let Err(error) = app.submit(Command::ToggleDoubleSize)
    {
        eprintln!("rustamp: could not start at double size: {error}");
    }
}

struct Windowed {
    app: App,
    /// The windows whose pixels changed since the last frame was presented
    /// (see [`WindowDirty`]): the tick paints only the windows in it, so a
    /// tick with no change draws nothing. The 16 ms tick stays the upper bound
    /// the clock-driven content needs.
    dirty: WindowDirty,
    /// The clipboard read behind `Ctrl`+`V` in a dialog's entry field (see
    /// [`crate::clipboard`]): the text is read on a worker thread and typed
    /// into the open field in [`Windowed::about_to_wait`].
    clipboard: Clipboard,
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
    /// The equalizer's `WM_COMMAND` seam: the window answers the commands
    /// addressed to it (S05 §1.1, §2.1: its shade is 275 x 14, its double
    /// size 550 x 232). Windows-only; without it the equalizer keeps its
    /// size and ignores the command.
    #[cfg(windows)]
    eq_commands: Option<classic_window_commands::WindowCommands>,
    /// The playlist editor window (`Winamp PE`, 275 x 116 at rest; S06, S01
    /// §4): a docked neighbour that follows the main window's bottom edge
    /// through the S05 geometry (see [`Windowed::apply_docking_positions`]).
    /// Created by the same shell and rail as the main window.
    playlist_window: Option<Arc<Window>>,
    playlist_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// The playlist editor's `WM_COMMAND` seam: the editor answers the
    /// commands addressed to it (S01 §1: its shade is 275 x 14; S05 §2.1: it
    /// is not affected by double size). Windows-only.
    #[cfg(windows)]
    playlist_commands: Option<classic_window_commands::WindowCommands>,
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
    /// The pointer over the dialog window, in client coordinates.
    dialog_cursor: (i32, i32),
    /// No fresh key press reached the dialog window yet.
    dialog_awaits_press: bool,
    /// The media library window: a **modern** window (native title bar,
    /// system font, its own chrome - see `crate::modern_shell`), created and
    /// dropped to match [`App::library_open`].
    ///
    /// It is not one of the classic windows: it is not skinned, it is
    /// resizable, and it is not part of the docked stack.
    library_window: Option<Arc<Window>>,
    library_surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// The library window's caption last applied.
    ///
    /// The caption carries the scan's progress, so it changes while a scan
    /// runs; the title is only set when it really changed.
    applied_library_title: Option<String>,
    /// A native media dialog (the file-open dialog or the shell's folder
    /// dialog) waiting to be shown.
    ///
    /// The request is a [`DialogKind`], so it carries the face (files or
    /// directory) and the requesting command's `play` flag with it.
    ///
    /// The native dialog is modal and runs its own message loop, so it must
    /// not be opened from inside a window procedure - the same rule the tray
    /// popup follows. The request is recorded here and drained once per tick
    /// in [`Windowed::drain_media_pick`].
    pending_media_pick: Option<DialogKind>,
    /// The preferences page the app asked for, opened from the tick for the
    /// same reason as the file pick (never from inside a window procedure).
    pending_prefs: Option<PrefsPage>,
    /// The load-playlist dialog waits for the tick.
    pending_playlist_pick: bool,
    /// Whether the main window is minimized; the other windows go with it.
    main_minimized: bool,
    /// The native preferences dialog while it is open.
    #[cfg(windows)]
    prefs_window: Option<rustamp_platform::PrefsWindow>,
    cursor: (i32, i32),
    eq_cursor: (i32, i32),
    left_down: bool,
    eq_left_down: bool,
    playlist_cursor: (i32, i32),
    /// The playlist editor's pointer position when a corner resize drag
    /// started, in client coordinates. `None` when not resizing.
    playlist_resize_start: Option<(i32, i32)>,
    /// The playlist editor's size when the corner resize drag started.
    playlist_resize_start_size: Option<(u32, u32)>,
    /// The media library window's pointer position, in client coordinates.
    library_cursor: (i32, i32),
    /// The skin's cursors as the operating system knows them, one per slot.
    ///
    /// The cache belongs to one skin load: it is emptied when the app
    /// loads another skin, so no cursor handle outlives its skin.
    cursors: SkinCursors,
    /// The skin load the windows' shapes were built for.
    ///
    /// A shape belongs to one skin: another skin's `region.txt` means
    /// another shape, so the shapes are built again when the skin changes.
    applied_region_skin: Option<u64>,
    /// The playlist list's double-click tracker (S03 §5: double-clicking an
    /// entry plays it). The rule is the shared one
    /// ([`crate::double_click`]).
    playlist_click: DoubleClick,
    modifiers: ModifiersState,
    /// The client size last requested, so repeat requests are skipped.
    applied_client: Option<(u32, u32)>,
    /// The equalizer's client size last requested.
    applied_eq_client: Option<(u32, u32)>,
    /// The playlist editor's client size last requested (its shade changes
    /// the height: 275 x 116 at rest, 275 x 14 shaded, S01 §1).
    applied_playlist_client: Option<(u32, u32)>,
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
    /// Builds the presenter around the app.
    ///
    /// `snapshot` is the display set the restore is clamped against; `None`
    /// means the screens could not be enumerated, and nothing is clamped.
    fn new(app: App, snapshot: Option<&DisplaySnapshot>) -> Self {
        let applied_on_top = app.always_on_top();
        let stack = Self::restore_stack(&app, snapshot);
        let cursors = SkinCursors::new(app.skin_generation());
        Self {
            app,
            dirty: WindowDirty::ALL,
            clipboard: Clipboard::new(),
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
            #[cfg(windows)]
            eq_commands: None,
            playlist_window: None,
            playlist_surface: None,
            #[cfg(windows)]
            playlist_commands: None,
            dialog_window: None,
            dialog_surface: None,
            dialog_id: None,
            dialog_kind: None,
            dialog_cursor: (0, 0),
            dialog_awaits_press: false,
            library_window: None,
            library_surface: None,
            applied_library_title: None,
            pending_media_pick: None,
            pending_prefs: None,
            pending_playlist_pick: false,
            main_minimized: false,
            #[cfg(windows)]
            prefs_window: None,
            cursor: (0, 0),
            eq_cursor: (0, 0),
            left_down: false,
            eq_left_down: false,
            playlist_cursor: (0, 0),
            playlist_resize_start: None,
            playlist_resize_start_size: None,
            library_cursor: (0, 0),
            cursors,
            applied_region_skin: None,
            playlist_click: DoubleClick::default(),
            modifiers: ModifiersState::empty(),
            applied_client: None,
            applied_eq_client: None,
            applied_playlist_client: None,
            applied_on_top,
            applied_main_visible: true,
            applied_title: None,
            stack,
        }
    }

    /// Rebuilds the docked stack from the saved window positions.
    ///
    /// The saved positions are the user's own and are restored verbatim; they
    /// are then clamped onto the screens that exist **now**
    /// ([`clamp_stack_to_screen`]). A monitor can vanish between two runs -
    /// it was unplugged, the resolution changed, the file was edited by hand -
    /// and a window that points at it would come back where the user cannot
    /// reach it. The clamp pulls such a window back until its title strip
    /// lies on a screen. A window that is already reachable - its title strip
    /// lies inside a work area - does not move at all, so the player does not
    /// jitter on every start.
    ///
    /// The **save** is not clamped: the configuration records the position the
    /// user actually had, and the screen set can change before the next
    /// start. The clamp belongs where the saved position meets the current
    /// screens, and that is the restore.
    ///
    /// Without a display set (the enumeration failed) nothing is clamped:
    /// there is no screen to place a window on.
    fn restore_stack(app: &App, snapshot: Option<&DisplaySnapshot>) -> DockedStack {
        // The state-derived stack (the windows' shade and double-size flags),
        // then the saved positions on top of it.
        let mut stack = app.docked_stack();
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
        match snapshot {
            Some(snapshot) => clamp_stack_to_screen(snapshot, stack),
            None => stack,
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
        if self.app.eq_state().double_size != self.stack.equalizer.double_size {
            self.stack
                .set_equalizer_double_size(self.app.eq_state().double_size);
        }
        if self.app.eq_state().windowshade != self.stack.equalizer.shaded {
            self.stack
                .set_equalizer_shaded(self.app.eq_state().windowshade);
        }
        if self.app.playlist_shaded() != self.stack.playlist.shaded {
            // A shaded editor is one row tall (S01 §1); the stack keeps the
            // edge connections and re-flows whatever hangs below it (S05 §3.3).
            self.stack.set_playlist_shaded(self.app.playlist_shaded());
        }
        let (playlist_width, playlist_height) = self.app.playlist_frame_size();
        self.stack.playlist.width = playlist_width;
        self.stack.playlist.height = playlist_height;
        self.apply_docking_positions();
        let mut shown = false;
        if let Some(window) = &self.window {
            if self.app.main_visible() != self.applied_main_visible {
                self.applied_main_visible = self.app.main_visible();
                window.set_visible(self.applied_main_visible);
                if self.applied_main_visible {
                    window.set_minimized(false);
                    window.focus_window();
                    shown = true;
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
        if shown {
            // The window is shown again: its shape belongs to the skin and
            // to the size the window has now.
            self.apply_main_region();
        }
    }

    /// Applies the skin's window shape to the player's windows.
    ///
    /// A shape belongs to one skin, one window size and one window state, so
    /// it is built again whenever one of them changes: the window is resized
    /// (which the shade and the double-size mode both do), another skin is
    /// loaded, or the window is shown again.
    fn sync_window_regions(&mut self) {
        self.applied_region_skin = Some(self.app.skin_generation());
        self.apply_main_region();
        self.apply_eq_region();
    }

    /// Cuts the main window to the shape the skin's region asks for.
    ///
    /// A skin without a `region.txt` (the base skin's case, S17 §7) leaves
    /// the window a plain rectangle: the region is cleared, not replaced by
    /// an empty one — an empty region would make the window invisible.
    fn apply_main_region(&mut self) {
        let raw = self.raw_main_window();
        if raw == 0 {
            return;
        }
        let state = self.app.state();
        let rects = crate::region::main_rects(
            self.app.skin.skin().region(),
            state.windowshade,
            state.double_size,
        );
        if !rustamp_platform::window_region::apply_window_region(raw, &rects) {
            eprintln!("rustamp: the main window's shape could not be applied");
        }
    }

    /// Cuts the equalizer window to the shape the skin's region asks for.
    ///
    /// The equalizer has a section of its own (S17 §7); a skin without one
    /// leaves the window a plain rectangle.
    fn apply_eq_region(&mut self) {
        let raw = self.raw_eq_window();
        if raw == 0 {
            return;
        }
        let state = self.app.eq_state();
        let rects = crate::region::equalizer_rects(
            self.app.skin.skin().region(),
            state.windowshade,
            state.double_size,
        );
        if !rustamp_platform::window_region::apply_window_region(raw, &rects) {
            eprintln!("rustamp: the equalizer window's shape could not be applied");
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
            Some(CoreWindowId::Gen(_)) => self.library_window.as_ref(),
            _ => None,
        };
        if let Some(window) = window {
            window.focus_window();
        }
    }

    /// Applies the commands the operating system posted to the classic
    /// windows.
    ///
    /// The native callback only queued them (see
    /// [`classic_window_commands`]): the message may have arrived on the
    /// thread that sent it, so the command is applied here, on the
    /// presenter's thread, through the same command path a menu or a
    /// shortcut uses. The windows are resized later in this tick
    /// ([`Windowed::sync_eq_window`], [`Windowed::sync_playlist_window`]),
    /// which read their size from the app.
    #[cfg(windows)]
    fn drain_window_commands(&mut self) {
        let mut commands = Vec::new();
        if let Some(seam) = &self.eq_commands {
            commands.extend(
                seam.take()
                    .into_iter()
                    .map(|command| (CoreWindowId::Equalizer, command)),
            );
        }
        if let Some(seam) = &self.playlist_commands {
            commands.extend(
                seam.take()
                    .into_iter()
                    .map(|command| (CoreWindowId::Playlist, command)),
            );
        }
        for (window, command) in commands {
            if let Err(error) = self.app.apply_window_command(window, command) {
                eprintln!("rustamp: the window command was refused: {error}");
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
                // The command seam goes first: it removes the native
                // subclass, which needs a live window.
                #[cfg(windows)]
                {
                    self.eq_commands = None;
                }
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
            // The equalizer answers the commands addressed to it (S05 §1.1,
            // §2.1). The seam is installed before the window is handed to the
            // rest of the presenter, so a command that arrives right after
            // the window exists is not lost.
            #[cfg(windows)]
            match classic_window_commands::WindowCommands::attach(&window) {
                Ok(commands) => self.eq_commands = Some(commands),
                Err(error) => {
                    eprintln!("rustamp: the equalizer window ignores window commands: {error}");
                }
            }
            self.eq_window = Some(window);
            self.eq_surface = Some(surface);
            self.applied_eq_client = Some((width, height));
            // The new window starts as a plain rectangle; the skin's shape
            // is cut out of it right away.
            self.apply_eq_region();
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

    /// The equalizer and the playlist editor go away with a minimized main
    /// window and come back with it (own choice: they have no taskbar entry of
    /// their own, and S05 does not measure it).
    fn follow_main_minimized(&mut self) {
        let minimized = self
            .window
            .as_ref()
            .is_some_and(|window| window.is_minimized() == Some(true));
        if minimized == self.main_minimized {
            return;
        }
        self.main_minimized = minimized;
        for window in [self.eq_window.as_ref(), self.playlist_window.as_ref()]
            .into_iter()
            .flatten()
        {
            window.set_visible(!minimized);
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
        self.app
            .set_window_shaded(CoreWindowId::Playlist, self.app.playlist_shaded());
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

    /// Shows the skin's cursor for the control under the pointer.
    ///
    /// `slot` is the slot the window's region maps to (see
    /// `rustamp_ui::cursor`) and `raw_window` the window's own handle. A
    /// slot the skin does not carry leaves the system's cursor in place: no
    /// `SetCursor` call is made at all.
    fn apply_cursor(&mut self, slot: CursorSlot, raw_window: isize) {
        // The cache belongs to one skin load; another skin empties it, so
        // no cursor of the old skin outlives it.
        self.cursors.sync(self.app.skin_generation());
        let file = self.app.skin.skin().cursor_slot(slot);
        let cursor = self.cursors.get(slot, file);
        match cursor_action(slot, file, cursor.is_some()) {
            CursorAction::Show(_) => {
                if let Some(cursor) = cursor {
                    rustamp_platform::cursor::set_window_cursor(raw_window, cursor);
                }
            }
            // No cursor for this slot: the system's own cursor stays.
            CursorAction::KeepSystem => {}
        }
    }

    /// Shows the main window's skin cursor for the control under the
    /// pointer.
    fn apply_main_cursor(&mut self) {
        let (x, y) = self.cursor;
        let widget = rustamp_ui::hit_test(&rustamp_ui::MainWindowLayout::CLASSIC, x, y);
        let slot = rustamp_ui::main_window_cursor(widget);
        let raw = self.raw_main_window();
        self.apply_cursor(slot, raw);
    }

    /// Shows the equalizer window's skin cursor for the control under the
    /// pointer.
    fn apply_eq_cursor(&mut self) {
        let (x, y) = self.eq_cursor;
        let widget = rustamp_ui::equalizer::hit_test(
            &rustamp_ui::EqualizerLayout::CLASSIC,
            x,
            y,
            self.app.eq_state().windowshade,
        );
        let slot = rustamp_ui::equalizer_cursor(widget);
        let raw = self.raw_eq_window();
        self.apply_cursor(slot, raw);
    }

    /// Shows the playlist window's skin cursor for the region under the
    /// pointer.
    fn apply_playlist_cursor(&mut self) {
        let (x, y) = self.playlist_cursor;
        let scale = self.app.playlist_scale() as i32;
        let slot = rustamp_ui::playlist_cursor_at(x / scale, y / scale);
        let raw = self.raw_playlist_window();
        self.apply_cursor(slot, raw);
    }

    /// Gives the pointer back to the operating system.
    ///
    /// The pointer keeps the cursor of the window it is over, so the
    /// system's own cursor is restored only when the pointer is over none
    /// of the player's windows: clicking the equalizer must not take the
    /// skin's cursor away from it.
    fn restore_system_cursor(&mut self) {
        let ours = [
            self.raw_main_window(),
            self.raw_eq_window(),
            self.raw_playlist_window(),
        ];
        if ours.contains(&rustamp_platform::cursor::pointer_window()) {
            return;
        }
        rustamp_platform::cursor::restore_cursor();
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

    fn request_redraw(&mut self) {
        // A request is made where a frame changes (input, focus, a window
        // that opened): it is the presenter's own half of the paint gate.
        self.dirty.main = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn request_redraw_eq(&mut self) {
        self.dirty.equalizer = true;
        if let Some(window) = &self.eq_window {
            window.request_redraw();
        }
    }

    fn request_playlist_redraw(&mut self) {
        self.dirty.playlist = true;
        if let Some(window) = &self.playlist_window {
            window.request_redraw();
        }
    }

    fn request_library_redraw(&mut self) {
        self.dirty.library = true;
        if let Some(window) = &self.library_window {
            window.request_redraw();
        }
    }

    fn paint(&mut self) {
        // Nothing to present without an OS window: a closed window is not
        // drawn (that used to be invisible work, since the gate only asks for
        // a frame when one changed).
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let image = self.app.render();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
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
        let Some(surface) = self.eq_surface.as_mut() else {
            return;
        };
        let image = self.app.render_eq();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
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
        let Some(surface) = self.playlist_surface.as_mut() else {
            return;
        };
        let image = self.app.render_playlist();
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
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

    /// Blits the media library frame ([`App::render_library`]) to its surface.
    fn paint_library(&mut self) {
        let Some(surface) = self.library_surface.as_mut() else {
            return;
        };
        let window = self.library_window.as_ref();
        let size = window.map_or((1, 1), |window| {
            let size = window.inner_size();
            (size.width.max(1), size.height.max(1))
        });
        let focused = window.is_some_and(|window| window.has_focus());
        let image = self.app.render_library_window(size, focused);
        let (Some(width), Some(height)) = (
            NonZeroU32::new(image.width()),
            NonZeroU32::new(image.height()),
        ) else {
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
    /// Shows the load-playlist dialog the app asked for (S06 §4), from the tick
    /// because it is modal, and loads what the user chose.
    fn drain_playlist_pick(&mut self) {
        if !std::mem::take(&mut self.pending_playlist_pick) {
            return;
        }
        #[cfg(windows)]
        match rustamp_platform::pick_playlist_file(
            self.raw_main_window(),
            DialogKind::LoadPlaylist.title(),
        ) {
            Ok(Some(path)) => {
                // A failure is reported by the load itself.
                let _ = self.app.load_playlist_file(&path, true);
            }
            Ok(None) => {}
            Err(error) => eprintln!("rustamp: no load-playlist dialog: {error}"),
        }
    }

    /// Shows a pending context menu at the pointer: the main window's title,
    /// time and seek menus and the playlist entry's (S03 §5).
    fn drain_context_menus(&mut self) {
        if let Some(kind) = self.app.take_context_menu() {
            let menu = self.app.native_context_menu(kind);
            let origin = self
                .window
                .as_ref()
                .and_then(|window| window.outer_position().ok());
            self.show_context_menu(self.raw_main_window(), menu, origin, self.cursor);
        }
        if self.app.take_pending_playlist_context().is_some() {
            let menu = rustamp_ui::menu::native_playlist_entry_menu();
            let origin = self
                .playlist_window
                .as_ref()
                .and_then(|window| window.outer_position().ok());
            self.show_context_menu(
                self.raw_playlist_window(),
                menu,
                origin,
                self.playlist_cursor,
            );
        }
    }

    /// Shows `menu` at `cursor` inside the window at `origin` and runs the
    /// chosen row.
    fn show_context_menu(
        &mut self,
        owner: isize,
        menu: rustamp_ui::menu::NativeMainMenu,
        origin: Option<PhysicalPosition<i32>>,
        cursor: (i32, i32),
    ) {
        #[cfg(windows)]
        {
            let Some(origin) = origin else {
                return;
            };
            let point = (origin.x + cursor.0, origin.y + cursor.1);
            match rustamp_platform::show_popup_menu(owner, &menu.rows, point) {
                Ok(None) => {}
                Ok(Some(id)) => {
                    if let Some(choice) = menu.choice_for_id(id)
                        && let Err(error) = self.app.activate_menu_choice(choice)
                    {
                        eprintln!("rustamp: menu item failed: {error}");
                    }
                }
                Err(error) => eprintln!("rustamp: no context menu: {error}"),
            }
        }
        #[cfg(not(windows))]
        let _ = (owner, menu, origin, cursor);
    }

    /// Shows the S04 §7 MISC popup; choices use the existing menu dispatch.
    /// Placement at the opening point is our choice, not a measured position.
    fn drain_playlist_misc_menu(&mut self) {
        if !self.app.take_playlist_misc_menu() {
            return;
        }
        #[cfg(windows)]
        {
            let Some(window) = self.playlist_window.as_ref() else {
                return;
            };
            let Ok(origin) = window.outer_position() else {
                return;
            };
            let menu = rustamp_ui::menu::native_playlist_misc_menu();
            let scale = self.app.playlist_scale() as i32;
            let height = self.app.playlist_frame_size().1 as i32;
            // The menu's owner is the window it opens from. Another owner takes
            // the activation, and the system paints a native caption over its
            // skinned title bar, which nothing repaints while the menu is up.
            match rustamp_platform::show_popup_menu(
                self.raw_playlist_window(),
                &menu.rows,
                (origin.x + 110 * scale, origin.y + height - 24 * scale),
            ) {
                Ok(None) => {}
                Ok(Some(id)) => match menu.choice_for_id(id) {
                    Some(choice) => {
                        if let Err(error) = self.app.activate_menu_choice(choice) {
                            eprintln!("rustamp: playlist menu item failed: {error}");
                        }
                    }
                    None => eprintln!("rustamp: the playlist menu reported an unknown id {id}"),
                },
                Err(error) => eprintln!("rustamp: no playlist MISC menu: {error}"),
            }
        }
        #[cfg(not(windows))]
        eprintln!("rustamp: a native popup menu is only implemented on Windows");
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

    /// Shows a pending native media dialog and adds what was chosen.
    ///
    /// The ADD button and the open-media actions ask for media. They used to
    /// open a window of ours that wanted a typed path or a drop onto it;
    /// Windows has a file-open dialog for exactly this, with multiple
    /// selection, and the shell has a folder dialog for the directory face,
    /// so the player uses them
    /// ([`rustamp_platform::pick_media_files`],
    /// [`rustamp_platform::pick_directory`]).
    ///
    /// Where the seam sits, and why here: the headless half keeps its model -
    /// a command still *requests* a media choice, which is what the app's
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
    /// A chosen **directory** contributes the media files of the tree below
    /// it (see [`crate::directory`]); the chosen directory itself is not an
    /// entry, and the scan runs in this tick. The location face keeps its
    /// own window: a URL is typed, not picked.
    fn drain_media_pick(&mut self) {
        let Some(dialog) = self.pending_media_pick.take() else {
            return;
        };
        let DialogKind::OpenMedia { kind, play } = dialog else {
            return;
        };
        let answer = match kind {
            OpenMediaKind::Files => match self.platform_media_files(dialog.title()) {
                Ok(paths) => MediaPick::Files(paths),
                Err(error) => MediaPick::FileFailed(error),
            },
            OpenMediaKind::Directory => match self.platform_media_directory(dialog.title()) {
                Ok(directory) => MediaPick::Directory(directory),
                Err(error) => MediaPick::DirectoryFailed(error),
            },
            // The location face is a typed URL, not a native dialog.
            OpenMediaKind::Location => return,
        };
        self.apply_media_pick(answer, play);
    }

    /// The files the user chose in the standard file-open dialog.
    ///
    /// An empty selection is a cancelled dialog, which is not an error.
    #[cfg(windows)]
    fn platform_media_files(
        &self,
        title: &str,
    ) -> Result<Vec<PathBuf>, rustamp_platform::FileDialogError> {
        rustamp_platform::pick_media_files(self.raw_main_window(), title, true)
    }

    /// No native file dialog on this platform yet (milestone M6).
    #[cfg(not(windows))]
    fn platform_media_files(
        &self,
        _title: &str,
    ) -> Result<Vec<PathBuf>, rustamp_platform::FileDialogError> {
        Err(rustamp_platform::FileDialogError::Unsupported)
    }

    /// The directory the user chose in the shell's folder dialog.
    ///
    /// `None` is a cancelled dialog, which is not an error.
    #[cfg(windows)]
    fn platform_media_directory(
        &self,
        title: &str,
    ) -> Result<Option<PathBuf>, rustamp_platform::FileDialogError> {
        rustamp_platform::pick_directory(self.raw_main_window(), title)
    }

    /// No native folder dialog on this platform yet (milestone M6).
    #[cfg(not(windows))]
    fn platform_media_directory(
        &self,
        _title: &str,
    ) -> Result<Option<PathBuf>, rustamp_platform::FileDialogError> {
        Err(rustamp_platform::FileDialogError::Unsupported)
    }

    /// Adds what a native media pick means to the playlist.
    ///
    /// * **Files**: the chosen files are appended. An empty selection is a
    ///   cancelled dialog and adds nothing.
    /// * **Directory**: the media files the chosen directory holds are
    ///   appended. `None` is a cancelled dialog and adds nothing.
    /// * **Failed**: the dialog failed; nothing is added and the failure is
    ///   reported.
    ///
    /// The `play` flag is the requesting command's, carried through
    /// untouched: the main window's open actions start the first added
    /// entry, the playlist's ADD only appends.
    fn apply_media_pick(&mut self, answer: MediaPick, play: bool) {
        match answer {
            MediaPick::Files(paths) => self.add_media_paths(paths, play),
            MediaPick::Directory(Some(directory)) => {
                let extensions = self.app.decodable_extensions();
                match crate::directory::media_files(&directory, &extensions) {
                    Ok(paths) if paths.is_empty() => {
                        // Nothing to add: say so, so a folder without media
                        // does not look like a button that does nothing.
                        self.app.note(format!(
                            "the chosen directory holds no media file: {}",
                            directory.display()
                        ));
                    }
                    Ok(paths) => self.add_media_paths(paths, play),
                    Err(error) => {
                        eprintln!("rustamp: the chosen directory could not be read: {error}")
                    }
                }
            }
            // Cancelled: the playlist stays as it is.
            MediaPick::Directory(None) => {}
            MediaPick::FileFailed(error) => eprintln!("rustamp: no file dialog: {error}"),
            MediaPick::DirectoryFailed(error) => {
                eprintln!("rustamp: no directory dialog: {error}")
            }
        }
    }

    /// Appends the chosen media files to the playlist.
    ///
    /// An empty list is a cancelled dialog: nothing is added, and the
    /// playlist model is left alone.
    fn add_media_paths(&mut self, paths: Vec<PathBuf>, play: bool) {
        if paths.is_empty() {
            return;
        }
        let command = crate::directory::add_command(&paths, play);
        if let Err(error) = self.app.submit(command) {
            eprintln!("rustamp: the chosen files could not be added: {error}");
        }
        self.app.sync_playlist_entries();
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

    /// Shows the native dialog for a Winamp configuration file and imports
    /// it (S12).
    ///
    /// The user picks the `winamp.ini` the reference player wrote, and the
    /// import reads it: the file is never written to. What the import could
    /// not honour is printed, so the user sees what was left out, and the
    /// preferences page shows the report of the import.
    ///
    /// The dialog runs its own modal message loop, so it is opened from the
    /// tick and never from a window procedure (the rule the tray popup
    /// follows).
    fn drain_winamp_ini_import(&mut self) {
        if !self.app.take_winamp_ini_import_request() {
            return;
        }
        #[cfg(windows)]
        {
            // The preferences dialog owns the file dialog when it is open:
            // the file dialog then opens in front of the page the user came
            // from, and not behind it.
            let owner = self
                .prefs_window
                .as_ref()
                .filter(|window| window.is_open())
                .map(|window| window.raw_handle())
                .unwrap_or_else(|| self.raw_main_window());
            let picked = match rustamp_platform::pick_winamp_ini_file(
                owner,
                "Import Winamp configuration",
            ) {
                Ok(picked) => picked,
                Err(error) => {
                    eprintln!("rustamp: no file dialog: {error}");
                    return;
                }
            };
            match self.app.finish_winamp_ini_import(picked.as_deref()) {
                // The user closed the dialog: nothing was imported.
                None => {}
                Some(crate::winamp_ini::ImportOutcome::Imported { path, report }) => {
                    println!(
                        "rustamp: imported {} ({})",
                        path.display(),
                        report.summary()
                    );
                    for line in report.lines() {
                        println!("rustamp: {line}");
                    }
                }
                Some(crate::winamp_ini::ImportOutcome::Failed { error }) => {
                    eprintln!("rustamp: the Winamp configuration was not imported: {error}")
                }
            }
            self.request_redraw();
        }
        #[cfg(not(windows))]
        eprintln!("rustamp: a native file dialog is only implemented on Windows");
    }

    /// Shows the native save dialog for the HTML playlist (S03 §2 `40292`).
    ///
    /// `Ctrl+Alt+G` in the playlist editor and the MISC menu's "Generate HTML
    /// playlist" row are the same command: the app records the request, the
    /// presenter asks where the document goes (the platform's HTML save
    /// dialog) and hands the answer back. A cancelled dialog writes nothing;
    /// the app notes it.
    fn drain_html_playlist(&mut self) {
        if !std::mem::take(&mut self.app.request_html_playlist) {
            return;
        }
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            match rustamp_platform::save_html_playlist_file(
                owner,
                "Generate HTML Playlist",
                "playlist.html",
            ) {
                Ok(Some(path)) => match self.app.finish_html_playlist(Some(path.as_path())) {
                    Ok(()) => println!("rustamp: HTML playlist written to {}", path.display()),
                    Err(error) => eprintln!("rustamp: the HTML playlist was not written: {error}"),
                },
                // The user closed the dialog: the app notes that nothing was
                // written.
                Ok(None) => {
                    if let Err(error) = self.app.finish_html_playlist(None) {
                        eprintln!("rustamp: the HTML playlist was not written: {error}");
                    }
                }
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
                        // The library page lists the media library's roots:
                        // they are read from the index when it is open, and
                        // from the database file when it is not.
                        if let Err(error) = self.app.refresh_library_roots() {
                            eprintln!("rustamp: the library roots could not be read: {error}");
                        }
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

    /// Shows the folder dialog the media library asked for.
    ///
    /// `Ctrl+O` in the library window asks for a folder to index, and the
    /// preferences page's `Add folder...` button asks for one too. The
    /// request is drained from the tick, never from a window procedure: the
    /// dialog runs its own modal message loop, exactly like the tray popup.
    /// A cancelled dialog adds nothing; a chosen folder is registered as a
    /// root and scanned.
    fn drain_library_folder_pick(&mut self) {
        if !self.app.take_library_folder_request() {
            return;
        }
        #[cfg(windows)]
        {
            match rustamp_platform::pick_directory(
                self.raw_main_window(),
                "Add a folder to the media library",
            ) {
                Ok(Some(folder)) => {
                    if let Err(error) = self.app.library_add_folder(folder) {
                        eprintln!("rustamp: the chosen folder could not be added: {error}");
                    }
                }
                // A cancelled dialog is not an error: nothing is added.
                Ok(None) => {}
                Err(error) => eprintln!("rustamp: no folder dialog: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            eprintln!("rustamp: a native folder dialog is only implemented on Windows");
        }
    }

    /// Asks for the settings of the playlist extraction and runs it.
    ///
    /// `Ctrl+Shift+X` in the media library asks for the path template and
    /// the target directory, and the extraction writes the selection into
    /// that directory (see [`crate::library_extract`]).
    ///
    /// The dialogs run their own modal message loops, so they are opened
    /// from the tick and never from a window procedure (the rule the tray
    /// popup follows). A cancelled dialog writes nothing: the first one is
    /// the configuration, the second one the question about an existing
    /// playlist.
    fn drain_library_extract(&mut self) {
        if !self.app.take_library_extract_request() {
            return;
        }
        #[cfg(windows)]
        {
            let owner = self.raw_main_window();
            let answered = {
                let last = self.app.extract_settings();
                rustamp_platform::ask_extract_settings(
                    owner,
                    &last.template,
                    &last.directory.display().to_string(),
                )
            };
            let settings = match answered {
                Ok(settings) => settings,
                Err(error) => {
                    eprintln!("rustamp: no extraction dialog: {error}");
                    None
                }
            };
            let mut ask = |question: &str| {
                match rustamp_platform::ask_yes_no_cancel(owner, question) {
                    Ok(Some(true)) => Some(rustamp_library::OverwriteAnswer::Yes),
                    Ok(Some(false)) => Some(rustamp_library::OverwriteAnswer::No),
                    // The question was not answered: nothing is written.
                    Ok(None) => None,
                    Err(error) => {
                        eprintln!("rustamp: no question dialog: {error}");
                        None
                    }
                }
            };
            // A cancelled configuration dialog is not an error: the app
            // writes nothing.
            if let Err(error) = self.app.library_extract(settings.as_ref(), &mut ask) {
                eprintln!("rustamp: the extraction failed: {error}");
            }
        }
        #[cfg(not(windows))]
        {
            eprintln!("rustamp: a native extraction dialog is only implemented on Windows");
        }
    }

    /// The main window's raw OS handle, or `0` when it has none yet.
    ///
    /// The one reader for the places that need the window itself rather than
    /// `winit`'s wrapper: the tray icon attach and the native dialogs.
    fn raw_main_window(&self) -> isize {
        raw_handle(self.window.as_deref())
    }

    /// The equalizer window's raw OS handle, or `0` when it has none.
    fn raw_eq_window(&self) -> isize {
        raw_handle(self.eq_window.as_deref())
    }

    /// The playlist window's raw OS handle, or `0` when it has none.
    fn raw_playlist_window(&self) -> isize {
        raw_handle(self.playlist_window.as_deref())
    }

    /// Paints the windows this tick changed with this tick's frame.
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
    /// The frame is only drawn for a window whose pixels changed (see
    /// [`WindowDirty`]): the app marks the windows its pump and events moved,
    /// and the presenter marks its own input paths ([`Windowed::request_redraw`]
    /// and its siblings). A window that is not marked keeps the frame it
    /// already has, so an idle player draws nothing. The tick itself stays the
    /// upper bound: the clock still marks the main window whenever its digits,
    /// marquee or analyser moved, exactly as often as before.
    ///
    /// `RedrawRequested` is still handled for repaints the OS itself asks for
    /// (an uncovered or resized window), and the input paths still request
    /// one so a press is reflected without waiting for the next tick.
    fn present_frames(&mut self) {
        if self.dirty.main {
            self.paint();
        }
        if self.dirty.equalizer {
            self.paint_eq();
        }
        if self.dirty.dialog {
            self.paint_dialog();
        }
        if self.dirty.playlist {
            self.paint_playlist();
        }
        if self.dirty.library {
            self.paint_library();
        }
        self.dirty = WindowDirty::NONE;
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

    /// Applies the library's caption to its window when it changed.
    ///
    /// The caption carries the scan's progress, so it changes while a scan
    /// runs; the title is only set when it really changed.
    fn sync_library_title(&mut self) {
        let title = self.app.library_title();
        if self.applied_library_title.as_deref() != Some(title.as_str())
            && let Some(window) = &self.library_window
        {
            window.set_title(&title);
            self.applied_library_title = Some(title);
        }
    }

    /// True when `id` is the equalizer window's id.
    fn is_eq_window(&self, id: WindowId) -> bool {
        self.eq_id == Some(id)
    }
    /// Hands the app's open dialog to the OS, when the OS has a dialog for
    /// it.
    ///
    /// A native dialog is not a window of ours: the request is recorded and
    /// drained from the tick (see [`Windowed::drain_media_pick`]), because
    /// this runs from window procedures too, and a modal dialog must not be
    /// opened from inside one.
    ///
    /// The app's own dialog is closed, so the two never answer at once.
    /// Returns whether the dialog was taken over; a dialog of ours
    /// ([`DialogRoute::OwnWindow`]) stays with the app.
    fn route_open_dialog(&mut self, kind: DialogKind) -> bool {
        match DialogRoute::for_dialog(kind) {
            DialogRoute::NativeFilePicker | DialogRoute::NativeDirectoryPicker => {
                self.app.close_dialog(kind);
                self.pending_media_pick = Some(kind);
                true
            }
            DialogRoute::NativePlaylistPicker => {
                self.app.close_dialog(kind);
                self.pending_playlist_pick = true;
                true
            }
            DialogRoute::NativePreferences { page } => {
                self.app.close_dialog(kind);
                self.pending_prefs = Some(page);
                true
            }
            DialogRoute::OwnWindow => false,
        }
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
        if self.route_open_dialog(kind) {
            return;
        }
        self.dialog_awaits_press = true;
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

    fn request_dialog_redraw(&mut self) {
        self.dirty.dialog = true;
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
    /// the open-media dialog, the load-playlist dialog, which loads the
    /// chosen playlist file, or the edit-entry box, which writes the changed
    /// tags; the preferences dialog is a no-op, its contents unmeasured),
    /// `Escape` (close), `Backspace`, `Tab` (the edit-entry box moves its
    /// focus with it), typed text (the open-media and load-playlist fields,
    /// the jump box and the edit-entry fields, see [`App::dialog_text`])
    /// and the arrows of the skin browser; the window closes itself when the
    /// app reports no open dialog afterwards.
    fn dialog_key_event(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        // The auto-repeat of the key that opened the dialog is not typing
        // either; the first fresh press ends the wait.
        if event.repeat && self.dialog_awaits_press {
            return;
        }
        self.dialog_awaits_press = false;
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
        // `Ctrl`+`V` pastes the clipboard into a dialog with an entry field
        // (the open-location face and the other text dialogs). The clipboard
        // is read on a worker thread (see [`crate::clipboard`]) and its text
        // reaches the field in [`Windowed::about_to_wait`], through the same
        // path a key press's text takes. The key is consumed here so it does
        // not also run its global shortcut (`Ctrl`+`V` is "stop after
        // current").
        if key == Key::Letter(b'V')
            && modifiers == Modifiers::CTRL
            && self.app.dialog_accepts_text()
        {
            self.clipboard.request();
            return;
        }
        // Modified keys stay global (the same key-event path as the main
        // window); plain keys go to the dialog first.
        if modifiers != Modifiers::NONE && !dialog_owns_key(key, modifiers) {
            self.key_event(event);
            return;
        }
        if self.app.dialog_key(key, modifiers) {
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
            WindowEvent::CursorMoved { position, .. } => {
                self.dialog_cursor = (position.x as i32, position.y as i32);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.dialog_cursor;
                if let (Some(window), Some(kind)) = (self.dialog_window.clone(), self.dialog_kind) {
                    if rustamp_ui::chrome::hits_close(window.inner_size().width, x, y) {
                        self.app.close_dialog(kind);
                        self.sync_dialog_window(event_loop);
                    } else if rustamp_ui::chrome::hits_title_bar(y) {
                        let _ = window.drag_window();
                    }
                }
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
            // The press that opened the dialog is still down when the new
            // window takes the focus, and the system reports held keys to it
            // as presses. They are not typed text.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
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
    ///
    /// `with_active` carries the focus rule of
    /// [`PresenterWindow::activates_on_open`]: the equalizer and the playlist
    /// editor are shown without being activated, so they leave the keyboard
    /// focus with the main window the way Winamp does.
    fn window_attributes(
        window: PresenterWindow,
        title: &str,
        size: (u32, u32),
        position: Option<(i32, i32)>,
    ) -> WindowAttributes {
        let attributes =
            rustamp_ui::shell::skinned_window_attributes(title, size, window.taskbar_role())
                .with_active(window.activates_on_open());
        #[cfg(windows)]
        let attributes = match window_class(window) {
            Some(class) => {
                use winit::platform::windows::WindowAttributesExtWindows as _;
                attributes.with_class_name(class)
            }
            None => attributes,
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

    /// Creates or drops the media library's OS window so it matches
    /// [`App::library_open`].
    ///
    /// The library window is a **modern** window: it is not skinned, so it
    /// is created through [`crate::modern_shell`] and not through the
    /// classic shell. Its client area follows the window's size, so a
    /// resize reaches the view (the number of rows it shows) and the page is
    /// loaded again.
    fn sync_library_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.library_open() {
            self.open_library_window(event_loop);
        } else if self.library_window.is_some() {
            self.library_surface = None;
            self.library_window = None;
            self.applied_library_title = None;
        }
    }

    /// Creates the media library window.
    ///
    /// The attributes come from the modern path
    /// ([`crate::modern_shell::modern_window_attributes`]): a decorated,
    /// resizable window without a taskbar entry of its own. The window is
    /// sized to the client area the app draws in
    /// ([`App::library_client_size`]).
    fn open_library_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.library_window.is_some() {
            return;
        }
        let (width, height) = self.app.library_client_size();
        let height = height + rustamp_ui::chrome::TITLE_BAR_HEIGHT;
        let title = self.app.library_title();
        // The window has to fit the screen, or its own close button lies
        // outside it. The room kept for the frame and the taskbar is our choice.
        let screen = event_loop
            .primary_monitor()
            .map(|monitor| monitor.size())
            .map_or((u32::MAX, u32::MAX), |size| (size.width, size.height));
        let width = width.min(screen.0.saturating_sub(32));
        let height = height.min(screen.1.saturating_sub(96));
        let attributes = crate::modern_shell::modern_window_attributes(
            &title,
            (width, height),
            TaskbarRole::Secondary,
        )
        .with_position(PhysicalPosition::new(8, 32));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the library window: {error}");
                return;
            }
        };
        let context = match softbuffer::Context::new(Arc::clone(&window)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("rustamp: could not create the library softbuffer context: {error}");
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("rustamp: could not create the library softbuffer surface: {error}");
                return;
            }
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            let _ = surface.resize(width, height);
        }
        self.library_window = Some(window);
        self.library_surface = Some(surface);
        self.applied_library_title = Some(title);
        self.request_library_redraw();
    }

    /// Events for the media library window, in client coordinates.
    ///
    /// The input goes to the app's library model ([`crate::library_window`]):
    /// the pointer picks a row or scrolls, the wheel scrolls, the keyboard
    /// runs the library's own map, and the typed text goes into the search
    /// field. Closing the window hides it (the same toggle the menu row and
    /// `Alt+L` use) and never quits the player.
    fn library_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Err(error) = self.app.close_library_window() {
                    eprintln!("rustamp: library close failed: {error}");
                }
                self.sync_library_window(event_loop);
            }
            WindowEvent::RedrawRequested => self.paint_library(),
            WindowEvent::Resized(size) => {
                if let Err(error) = self.app.library_resize(library_body_size(size)) {
                    eprintln!("rustamp: the library resize failed: {error}");
                }
                self.request_library_redraw();
            }
            WindowEvent::Focused(_) => self.request_library_redraw(),
            WindowEvent::ScaleFactorChanged { .. } => {
                // A DPI change moves the window to another monitor: the
                // client area follows it, and the number of rows the view
                // shows has to follow the client area.
                if let Some(window) = &self.library_window {
                    let size = window.inner_size();
                    if let Err(error) = self.app.library_resize(library_body_size(size)) {
                        eprintln!("rustamp: the library resize failed: {error}");
                    }
                }
                self.request_library_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.library_cursor = (position.x as i32, position.y as i32);
                if let Some(window) = &self.library_window {
                    let edge = resize_edge(window.inner_size(), self.library_cursor);
                    window.set_cursor(edge.map_or(winit::window::CursorIcon::Default, |edge| {
                        winit::window::CursorIcon::from(edge)
                    }));
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.library_cursor;
                if let Some(window) = self.library_window.clone() {
                    let size = window.inner_size();
                    if rustamp_ui::chrome::hits_close(size.width, x, y) {
                        if let Err(error) = self.app.close_library_window() {
                            eprintln!("rustamp: library close failed: {error}");
                        }
                        self.sync_library_window(event_loop);
                        return;
                    }
                    if let Some(edge) = resize_edge(size, (x, y)) {
                        let _ = window.drag_resize_window(edge);
                        return;
                    }
                    if rustamp_ui::chrome::hits_title_bar(y) {
                        let _ = window.drag_window();
                        return;
                    }
                }
                let y = y - rustamp_ui::chrome::TITLE_BAR_HEIGHT as i32;
                if let Err(error) = self.app.library_click(x, y) {
                    eprintln!("rustamp: library pointer input failed: {error}");
                }
                self.request_library_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, lines) => lines as isize,
                    winit::event::MouseScrollDelta::PixelDelta(pixels) => {
                        (pixels.y as isize / LIBRARY_LINE_HEIGHT as isize).clamp(-1, 1)
                    }
                };
                if let Err(error) = self.app.library_scroll(-rows) {
                    eprintln!("rustamp: library scroll failed: {error}");
                }
                self.request_library_redraw();
            }
            // A key still held when the window takes the focus is reported as
            // a press; it was meant for the window it was pressed in.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
            WindowEvent::KeyboardInput { event, .. } => self.library_key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            _ => {}
        }
    }

    /// A key press while the media library window is focused.
    ///
    /// The text of the key goes into the search field and the key itself
    /// runs the library's own map; the map decides what a key means, so the
    /// text never reaches the list.
    fn library_key_event(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        // `Alt+L` closes the window it opened (the toggle of the main window).
        if self.modifiers.alt_key()
            && let Some(Key::Letter(b'L')) = map_key(event)
        {
            if let Err(error) = self.app.key_down(Key::Letter(b'L'), Modifiers::ALT) {
                eprintln!("rustamp: shortcut dispatch failed: {error}");
            }
            return;
        }
        if let Some(text) = &event.text
            && let Err(error) = self.app.library_text(text)
        {
            eprintln!("rustamp: the library search failed: {error}");
        }
        let Some(key) = map_key(event) else {
            return;
        };
        let modifiers = map_modifiers(self.modifiers);
        match self.app.library_key_down(key, modifiers) {
            Ok(commands) if commands.is_empty() => {}
            Ok(commands) => println!("rustamp: library shortcut {key:?} -> {commands:?}"),
            Err(error) => eprintln!("rustamp: library shortcut dispatch failed: {error}"),
        }
        self.request_library_redraw();
    }

    /// Creates or drops the playlist editor's OS window so it matches
    /// [`App::playlist_open`], the way [`Windowed::sync_eq_window`] follows
    /// the equalizer: the PL button, the menu row, the playlist's own Close
    /// and its OS close box all flip that one flag. The window's client area
    /// follows the editor's shade state ([`App::playlist_frame_size`]), so
    /// the shade button resizes the OS window and not just the painted
    /// surface.
    fn sync_playlist_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.playlist_open() {
            self.open_playlist_window(event_loop);
            // During a corner resize drag, the size is driven by the mouse
            // move handler; skip the config-based sync to avoid fighting it.
            if self.playlist_resize_start.is_none() {
                let desired = self.app.playlist_frame_size();
                if self.applied_playlist_client != Some(desired)
                    && let Some(window) = &self.playlist_window
                {
                    let _ = window.request_inner_size(PhysicalSize::new(desired.0, desired.1));
                    self.applied_playlist_client = Some(desired);
                }
            }
        } else if self.playlist_window.is_some() {
            // The command seam goes first: it removes the native subclass,
            // which needs a live window.
            #[cfg(windows)]
            {
                self.playlist_commands = None;
            }
            self.playlist_surface = None;
            self.playlist_window = None;
            self.playlist_click.clear();
            self.playlist_resize_start = None;
            self.playlist_resize_start_size = None;
            self.applied_playlist_client = None;
        }
    }

    /// Creates the playlist editor window with the same shell as the main
    /// window (S06, S01 §4): a non-resizable `winit` window with its own
    /// `softbuffer` surface, sized to the spec'd client area
    /// ([`App::playlist_frame_size`]: 275 x 116 at rest, 275 x 14 shaded,
    /// S01 §1), under the same [`ensure_gui_allowed`] rail (checked once in
    /// [`run`], before either window exists).
    fn open_playlist_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.playlist_window.is_some() {
            return;
        }
        let (width, height) = self.app.playlist_frame_size();
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
        // The playlist editor answers the commands addressed to it (S01 §1:
        // its shade is 275 x 14). The seam is installed before the window is
        // handed to the rest of the presenter, so a command that arrives
        // right after the window exists is not lost.
        #[cfg(windows)]
        match classic_window_commands::WindowCommands::attach(&window) {
            Ok(commands) => self.playlist_commands = Some(commands),
            Err(error) => {
                eprintln!("rustamp: the playlist window ignores window commands: {error}");
            }
        }
        self.playlist_window = Some(window);
        self.playlist_surface = Some(surface);
        self.applied_playlist_client = Some((width, height));
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
                // Keep the client aligned with the app's configured size and
                // optional playlist scale, including the shade band.
                let (width, height) = self.app.playlist_frame_size();
                if size.width < width || size.height < height {
                    // Never settle below the layout (the DPI-change floor).
                    self.applied_playlist_client = None;
                }
                // The size is NOT persisted here: only a corner drag writes
                // pe_width/pe_height (the release handler). Persisting every
                // resize made the stored size feed the next request and the
                // player never reached another tick (resize storm, 2026-10-22).
                self.sync_playlist_window(event_loop);
                self.request_playlist_redraw();
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                let (width, height) = self.app.playlist_frame_size();
                let _ = inner_size_writer.request_inner_size(PhysicalSize::new(width, height));
                self.applied_playlist_client = Some((width, height));
                self.request_playlist_redraw();
            }
            WindowEvent::DroppedFile(path) => self.drop_file(path),
            WindowEvent::Focused(focused) => {
                self.app.set_playlist_focused(focused);
                if focused {
                    // Winamp treats the three classic windows as one player:
                    // activating any of them raises the whole group.
                    let others = [self.raw_main_window(), self.raw_eq_window()];
                    raise_classic_group(self.raw_playlist_window(), &others);
                } else {
                    // The pointer keeps the cursor it had; give the system
                    // its own back when the window loses the focus.
                    self.restore_system_cursor();
                    self.app.playlist_mouse_leave();
                    self.playlist_click.clear();
                }
                self.request_playlist_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.app.playlist_wheel(wheel_notches(delta));
                self.request_playlist_redraw();
            }
            // A key still held when the window takes the focus is reported as
            // a press; it was meant for the window it was pressed in.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
            WindowEvent::KeyboardInput { event, .. } => self.playlist_key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                // Keep physical coordinates for the resize gesture.
                self.playlist_cursor = (position.x as i32, position.y as i32);
                // Playlist corner resize drag: update window size during drag.
                if let (Some((start_x, start_y)), Some((start_w, start_h))) =
                    (self.playlist_resize_start, self.playlist_resize_start_size)
                {
                    let dx = self.playlist_cursor.0 - start_x;
                    let dy = self.playlist_cursor.1 - start_y;
                    // Minimum size is the classic 275 x 116 (S01 §1, S05 §2.1).
                    let scale = self.app.playlist_scale();
                    let new_w = (start_w as i32 + dx).clamp((275 * scale) as i32, 4096) as u32;
                    let new_h = (start_h as i32 + dy).clamp((116 * scale) as i32, 4096) as u32;
                    self.app.set_playlist_size(new_w, new_h);
                    if let Some(window) = &self.playlist_window {
                        let (width, height) = self.app.playlist_frame_size();
                        let _ = window.request_inner_size(PhysicalSize::new(width, height));
                    }
                } else {
                    self.apply_playlist_cursor();
                    self.app
                        .playlist_mouse_move(self.playlist_cursor.0, self.playlist_cursor.1);
                    if self.app.playlist().drag_insertion().is_some() {
                        self.playlist_click.clear();
                    }
                }
                self.request_playlist_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                // The skin's cursor only holds while the pointer is over
                // this window; the system's own cursor takes over.
                self.restore_system_cursor();
                self.app.playlist_mouse_leave();
                self.playlist_click.clear();
                self.request_playlist_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                // S03 §5: a right click on a playlist entry selects that
                // entry and asks for the playlist's context menu. The app
                // records the request (blocked on S04: the menu's rows and
                // wording are S04's) and the tick in `about_to_wait` takes it
                // once; a press that names no entry does nothing.
                let (x, y) = self.playlist_cursor;
                if let Err(error) = self.app.playlist_right_mouse_down(x, y) {
                    eprintln!("rustamp: playlist right-click failed: {error}");
                }
                self.request_playlist_redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.playlist_cursor;
                let scale = self.app.playlist_scale() as i32;
                let (layout_x, layout_y) = (x / scale, y / scale);
                if state == ElementState::Released && self.playlist_resize_start.is_some() {
                    self.playlist_resize_start = None;
                    self.playlist_resize_start_size = None;
                    self.applied_playlist_client = None;
                    self.sync_playlist_window(event_loop);
                    self.request_playlist_redraw();
                    return;
                }
                if state == ElementState::Pressed
                    // S03 §5: a double click on the title bar toggles the
                    // windowshade. It is asked first, so the double click
                    // does not start a window drag.
                    && self.title_bar_double_click(CoreWindowId::Playlist, layout_x, layout_y)
                {
                    self.playlist_click.clear();
                    self.sync_window_state();
                    self.sync_playlist_window(event_loop);
                    self.apply_docking_positions();
                    self.request_playlist_redraw();
                    self.request_redraw();
                    return;
                }
                if state == ElementState::Pressed
                    && (0..rustamp_ui::playlist_window::TITLE_BAR_HEIGHT).contains(&layout_y)
                    && rustamp_ui::playlist_window::hit_test_title_bar(
                        layout_x - (self.app.playlist_frame_size().0 as i32 / scale - 275),
                        layout_y,
                    )
                    .is_none()
                    && let Some(window) = &self.playlist_window
                    && window.drag_window().is_ok()
                {
                    self.playlist_click.clear();
                    return;
                }
                // Playlist corner resize drag (Bug 1): drag from the bottom-right
                // corner resizes the playlist editor. The grip is a small region
                // (16 x 16 px) at the corner; the minimum size is the classic
                // 275 x 116 (S01 §1, S05 §2.1). The size persists as
                // pe_width/pe_height (S12 §3.4).
                const PLAYLIST_RESIZE_GRIP: i32 = 16;
                let (pl_width, pl_height) = self.app.playlist_frame_size();
                let in_resize_grip = x >= pl_width as i32 - PLAYLIST_RESIZE_GRIP
                    && y >= pl_height as i32 - PLAYLIST_RESIZE_GRIP;
                if in_resize_grip && !self.app.playlist_shaded() && state == ElementState::Pressed {
                    self.playlist_resize_start = Some((x, y));
                    self.playlist_resize_start_size = Some((pl_width, pl_height));
                    self.playlist_click.clear();
                    return;
                }
                // Row dragging is an own choice while S06 §8 Q6 stays open.
                let result = match state {
                    ElementState::Pressed => {
                        let now = self.app.now_ms();
                        // The double-click gesture is the list's (S03 §5:
                        // double-clicking an entry plays it), so a title-bar
                        // press is never swallowed as one - the shade button
                        // must toggle on every click.
                        let is_double = self.app.playlist().contains_list_point(layout_x, layout_y)
                            && self.playlist_click.is_double_click(now, x, y);
                        if is_double {
                            self.playlist_click.clear();
                            self.app.playlist_double_click(x, y)
                        } else {
                            self.playlist_click.remember(now, x, y);
                            // The modifiers choose the list gesture: the
                            // shift-range and the ctrl-toggle need them.
                            self.app
                                .playlist_mouse_down(x, y, map_modifiers(self.modifiers))
                        }
                    }
                    ElementState::Released => self.app.playlist_mouse_up(x, y),
                };
                if let Err(error) = result {
                    eprintln!("rustamp: playlist pointer input failed: {error}");
                }
                // The title-bar Close may have hidden the editor, ADD may
                // have opened the open-media dialog, and the shade button may
                // have made the editor one row tall (S01 §1) - the window
                // state sync re-flows the docked stack with it (S05 §3.3).
                self.sync_window_state();
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

    /// S03 §5: a double click on a window's title bar toggles its
    /// windowshade.
    ///
    /// The press is asked of the app first, so the double click never starts
    /// a window drag: the drag belongs to the single click, the double click
    /// to the shade. Returns whether the press completed the double click,
    /// in which case the shade has been toggled.
    fn title_bar_double_click(&mut self, window: CoreWindowId, x: i32, y: i32) -> bool {
        let now = self.app.now_ms();
        match self.app.title_bar_double_click(window, x, y, now) {
            Ok(commands) => !commands.is_empty(),
            Err(error) => {
                eprintln!("rustamp: title-bar double click failed: {error}");
                false
            }
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

    /// Prints the notes the app raised.
    ///
    /// The app's own rule ([`App::print_notes`]): the player's prefix on the
    /// standard error, the notes in the order they were raised. The headless
    /// run prints them the same way, so a diagnostic reads the same in
    /// either mode — and the queue is drained, so a note is printed **once**.
    fn print_notes(&mut self) {
        self.app.print_notes();
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
        )
        // Discovery of a visible main window must not expose a bare winit
        // procedure. Install external control before surface/tray setup and
        // before sync_window_state can show the window.
        .with_visible(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("rustamp: could not create the window: {error}");
                event_loop.exit();
                return;
            }
        };
        #[cfg(windows)]
        match crate::external_control::native::ExternalControl::attach(
            Arc::clone(&window),
            self.app.host_player_state(),
        ) {
            Ok(control) => {
                self.external_control = Some(control);
                eprintln!("rustamp: external-control handler installed (window still hidden)");
            }
            Err(error) => {
                // A discoverable player without its answering handler is not
                // a usable external-control endpoint. Do not silently show it.
                eprintln!("rustamp: external control unavailable: {error}");
                event_loop.exit();
                return;
            }
        }
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
        self.applied_main_visible = false;
        if let Some(window) = &self.window {
            self.main_id = Some(window.id());
        }
        #[cfg(windows)]
        self.attach_tray();
        self.sync_window_state();
        // The skin's shape is cut out of the main window; the equalizer
        // gets its own shape when it is created below.
        self.sync_window_regions();
        self.sync_eq_window(event_loop);
        self.request_redraw();
        self.request_redraw_eq();
        self.sync_dialog_window(event_loop);
        self.request_dialog_redraw();
        self.sync_playlist_window(event_loop);
        eprintln!("rustamp: main-window startup complete; awaiting first full UI tick");
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
        if self
            .library_window
            .as_ref()
            .is_some_and(|window| window.id() == id)
        {
            self.library_event(event_loop, event);
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
                self.follow_main_minimized();
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
                // The shape belongs to one size: a resized window is cut to
                // the skin's shape again.
                self.apply_main_region();
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
                if focused {
                    // Winamp treats the three classic windows as one player:
                    // activating any of them raises the whole group.
                    let others = [self.raw_eq_window(), self.raw_playlist_window()];
                    raise_classic_group(self.raw_main_window(), &others);
                } else {
                    // The pointer keeps the cursor it had; give the system
                    // its own back when the window loses the focus.
                    self.restore_system_cursor();
                }
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Err(error) = self.app.main_wheel(wheel_notches(delta)) {
                    eprintln!("rustamp: the volume change failed: {error}");
                }
                self.request_redraw();
            }
            // A key still held when the window takes the focus is reported as
            // a press; it was meant for the window it was pressed in.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
            WindowEvent::KeyboardInput { event, .. } => self.key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = self.to_layout(position);
                self.apply_main_cursor();
                if self.left_down {
                    let (x, y) = self.cursor;
                    if let Err(error) = self.app.mouse_move(x, y) {
                        eprintln!("rustamp: pointer move failed: {error}");
                    }
                    self.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // The skin's cursor only holds while the pointer is over
                // this window; the system's own cursor takes over.
                self.restore_system_cursor();
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
                        // S03 §5: a double click on the title bar toggles
                        // the windowshade. It is asked first, so the double
                        // click does not start a window drag.
                        if self.title_bar_double_click(CoreWindowId::Main, x, y) {
                            Ok(Vec::new())
                        } else {
                            // Easy-move (W10 finding F5): a press no control
                            // owns moves the window instead of reaching the
                            // machine.
                            match self.try_easy_drag(x, y) {
                                Some(()) => Ok(Vec::new()),
                                None => self.app.mouse_down(x, y),
                            }
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
        // The commands the operating system posted to the equalizer and the
        // playlist editor (S05 §1.1, §2.1; S01 §1) are applied before the
        // windows are resized below, so the resize follows in the same tick.
        #[cfg(windows)]
        self.drain_window_commands();
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
        self.drain_context_menus();
        self.drain_playlist_pick();
        // A `--no-audio` run has no device pulling on its own thread, so the
        // virtual clock is driven here, once per frame, exactly as
        // `run_headless` drives it headless. With a real device this is a
        // no-op.
        self.app.drive_null_clock();
        if let Err(error) = self.app.pump() {
            eprintln!("rustamp: pump failed: {error}");
        }
        // The text the clipboard worker read for `Ctrl`+`V` (see
        // [`crate::clipboard`]) reaches the open dialog's field through the
        // same path a key press's text takes, so paste and typing cannot
        // drift. An empty or unreadable clipboard leaves nothing to take — the
        // press is a silent no-op — and a dialog that closed or lost its
        // field in the meantime takes nothing either. Only a text that really
        // landed marks the dialog's frame.
        if let Some(text) = self.clipboard.take()
            && self.app.dialog_text(&text)
        {
            self.dirty.dialog = true;
        }
        // The windows the app's commands, events and clock moved (see
        // [`WindowDirty`]); the presenter's own marks came from input and
        // window events. The app computes its half here, on this request.
        self.dirty.merge(self.app.take_window_dirty());
        #[cfg(windows)]
        if let Some(control) = &self.external_control {
            control.publish(self.app.host_player_state());
        }
        // The notes the app raised, printed by the app's own rule: the
        // headless run prints them the same way (see `App::print_notes`).
        self.print_notes();
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
        // The media library follows its open flag; it is a modern window, so
        // it is not part of the docked stack.
        self.sync_library_window(event_loop);
        // Its caption carries the scan's progress.
        self.sync_library_title();
        self.apply_docking_positions();
        // The frame of this tick, painted here rather than asked for: see
        // [`Windowed::present_frames`].
        self.present_frames();
        // Outside every window procedure, like the tray popup: both of
        // these run their own modal message loop.
        self.drain_main_menu();
        self.drain_clutter_menu();
        self.drain_playlist_misc_menu();
        self.drain_eq_menu();
        self.drain_media_pick();
        self.drain_winamp_ini_import();
        self.drain_save_playlist();
        self.drain_html_playlist();
        self.drain_prefs();
        self.drain_library_folder_pick();
        self.drain_library_extract();
        // The drains applied commands of their own (a picked file, a
        // preference): their marks stay in the app's mask and reach the next
        // tick's paint, which is the frame after the one just presented.
        // After every window has been synced, so a focus request for a window
        // that opened this frame finds its OS window.
        self.apply_focus_requests();
        // An error the user must see goes to the native title (BETA-04).
        self.sync_title();
        // A skin loaded in this tick replaces the cursors: the cache of the
        // old skin is emptied, so no cursor handle outlives its skin.
        self.cursors.sync(self.app.skin_generation());
        // The same skin brings another `region.txt`: the windows' shapes
        // follow it.
        if self.applied_region_skin != Some(self.app.skin_generation()) {
            self.sync_window_regions();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
        #[cfg(windows)]
        if let Some(control) = &mut self.external_control
            && let Err(error) = control.mark_ready()
        {
            eprintln!("rustamp: external-control readiness unavailable: {error}");
            event_loop.exit();
        }
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
                // The shape belongs to one size: a resized window is cut
                // to the skin's shape again.
                self.apply_eq_region();
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
                if focused {
                    // Winamp treats the three classic windows as one player:
                    // activating any of them raises the whole group.
                    let others = [self.raw_main_window(), self.raw_playlist_window()];
                    raise_classic_group(self.raw_eq_window(), &others);
                } else {
                    // The pointer keeps the cursor it had; give the system
                    // its own back when the window loses the focus.
                    self.restore_system_cursor();
                }
                self.request_redraw_eq();
            }
            // A key still held when the window takes the focus is reported as
            // a press; it was meant for the window it was pressed in.
            WindowEvent::KeyboardInput {
                is_synthetic: true, ..
            } => {}
            WindowEvent::KeyboardInput { event, .. } => self.eq_key_event(&event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.eq_cursor = self.to_eq_layout(position);
                self.apply_eq_cursor();
                if self.eq_left_down {
                    let (x, y) = self.eq_cursor;
                    if let Err(error) = self.app.eq_mouse_move(x, y) {
                        eprintln!("rustamp: equalizer pointer move failed: {error}");
                    }
                    self.request_redraw_eq();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // The skin's cursor only holds while the pointer is over
                // this window; the system's own cursor takes over.
                self.restore_system_cursor();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.eq_cursor;
                if state == ElementState::Pressed
                    // S03 §5: a double click on the title bar toggles the
                    // windowshade. It is asked first, so the double click
                    // does not start a window drag.
                    && self.title_bar_double_click(CoreWindowId::Equalizer, x, y)
                {
                    self.eq_left_down = false;
                    self.sync_window_state();
                    self.sync_eq_window(event_loop);
                    self.request_redraw_eq();
                    return;
                }
                if state == ElementState::Pressed
                    && rustamp_ui::equalizer::hit_test(
                        &rustamp_ui::EqualizerLayout::CLASSIC,
                        x,
                        y,
                        self.app.eq_state().windowshade,
                    ) == Some(rustamp_ui::EqWidget::TitleBar)
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

/// Whether a key press belongs to the open dialog instead of the global
/// shortcut path.
///
/// A modified key is a shortcut and stays global — except `Tab`: the
/// edit-entry box moves its focus with `Tab` and `Shift`+`Tab`, and a
/// `Shift`+`Tab` that went to the global path would never come back.
/// `Ctrl`+`Tab` stays global: it is the "switch to the next window" row
/// (S03 §1 id 40063).
/// The media library's own part of a window of `size`: all but the title bar.
fn library_body_size(size: PhysicalSize<u32>) -> (u32, u32) {
    (
        size.width.max(1),
        size.height
            .saturating_sub(rustamp_ui::chrome::TITLE_BAR_HEIGHT)
            .max(1),
    )
}

/// The edge of a borderless resizable window a press at `(x, y)` resizes: the
/// sides and the bottom, in a band of a few pixels (own choice). The top belongs
/// to the title bar.
fn resize_edge(
    size: PhysicalSize<u32>,
    (x, y): (i32, i32),
) -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection as Edge;
    const BAND: i32 = 5;
    if y < rustamp_ui::chrome::TITLE_BAR_HEIGHT as i32 {
        return None;
    }
    let left = x < BAND;
    let right = x >= size.width as i32 - BAND;
    let bottom = y >= size.height as i32 - BAND;
    match (left, right, bottom) {
        (true, _, true) => Some(Edge::SouthWest),
        (_, true, true) => Some(Edge::SouthEast),
        (true, _, false) => Some(Edge::West),
        (_, true, false) => Some(Edge::East),
        (_, _, true) => Some(Edge::South),
        _ => None,
    }
}

/// A wheel event as whole notches; a touchpad's small step still counts as one.
fn wheel_notches(delta: winit::event::MouseScrollDelta) -> i32 {
    let amount = match delta {
        winit::event::MouseScrollDelta::LineDelta(_, lines) => lines,
        winit::event::MouseScrollDelta::PixelDelta(pixels) => pixels.y as f32 / 120.0,
    };
    if amount == 0.0 {
        0
    } else if amount.abs() < 1.0 {
        amount.signum() as i32
    } else {
        amount.round() as i32
    }
}

fn dialog_owns_key(key: Key, modifiers: Modifiers) -> bool {
    key == Key::Tab && !modifiers.ctrl() && !modifiers.alt()
}

/// The window's raw OS handle, or `0` when it has none.
///
/// Only the Win32 handle is of use (the platform's cursor API takes it);
/// every other platform answers `0`.
fn raw_handle(window: Option<&Window>) -> isize {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(window) = window else {
        return 0;
    };
    match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
        _ => 0,
    }
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

/// The `WM_COMMAND` seam of the classic windows that are not the main one.
///
/// The main window answers a posted `WM_COMMAND` through the external
/// control ([`crate::external_control`]). The equalizer and the playlist
/// editor had **no** answer at all: `winit` lets the message fall through
/// to `DefWindowProc`, the window keeps its size, and a windowshade or a
/// double-size command addressed to them changed nothing (S05 §1.1: the
/// equalizer's shade is 275 x 14 and its double size 550 x 232; S01 §1:
/// the playlist editor's shade is 275 x 14).
///
/// The reference applies such a command to the window it was addressed to
/// (S01 §1: the equalizer's shade has its own flag, `eq_ws`, and the
/// playlist editor's its own, `pe_height_ws`, beside the main window's
/// `windowshade`), so the subclass below answers the two ids that carry a
/// window's own geometry: 40064 (windowshade) and 40165 (double size),
/// S03 §1.
///
/// The native callback only **queues** the command. The message may arrive
/// on the thread that sent it — a `SendMessage` from another process runs
/// the receiver's window procedure on the *sender's* thread — so the queue
/// is shared and locked, and the presenter applies the command on its own
/// thread (see [`Windowed::drain_window_commands`]).
#[cfg(windows)]
#[allow(unsafe_code)] // the Win32 subclass boundary only; the rest of the app denies unsafe.
mod classic_window_commands {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use crate::app::WindowCommand;

    /// The message the reference's commands arrive as (S03 §1).
    const WM_COMMAND: u32 = 0x0111;
    /// The message the operating system sends before it destroys a window.
    const WM_NCDESTROY: u32 = 0x0082;

    /// The callback type `SetWindowSubclass` takes.
    type SubclassProc =
        unsafe extern "system" fn(*mut c_void, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(
            hwnd: *mut c_void,
            callback: SubclassProc,
            id: usize,
            data: usize,
        ) -> i32;
        fn RemoveWindowSubclass(hwnd: *mut c_void, callback: SubclassProc, id: usize) -> i32;
        fn DefSubclassProc(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
    }

    /// The subclass's id inside a window's subclass chain.
    ///
    /// One id for every window: the chain belongs to one window, so the id
    /// only has to be unique there.
    const SUBCLASS_ID: usize = 0x5241_4d50; // "RAMP"

    /// The shared state of one window's seam.
    struct Seam {
        /// The commands the operating system posted, waiting for the
        /// presenter to apply them.
        pending: Mutex<Vec<WindowCommand>>,
        /// Whether the subclass is still installed.
        ///
        /// The window's own destruction removes it (see the callback), and
        /// the seam must not remove it a second time.
        attached: AtomicBool,
    }

    impl Seam {
        /// Queues a command the operating system posted.
        fn push(&self, command: WindowCommand) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.push(command);
            }
        }

        /// Takes the queued commands, in the order they arrived.
        fn take(&self) -> Vec<WindowCommand> {
            match self.pending.lock() {
                Ok(mut pending) => std::mem::take(&mut *pending),
                Err(_) => Vec::new(),
            }
        }
    }

    /// The command a window message carries for one of the player's classic
    /// windows, if any.
    ///
    /// `WM_COMMAND` carries the id in the low word of `wparam`; the high
    /// word is the notification code, which the two ids this player answers
    /// do not use (S03 §1).
    fn command_from_message(message: u32, wparam: usize) -> Option<WindowCommand> {
        if message != WM_COMMAND {
            return None;
        }
        WindowCommand::from_command_id((wparam & 0xffff) as u32)
    }

    /// The native callback: it queues the command and lets the rest of the
    /// message chain run.
    ///
    /// # Safety
    ///
    /// `data` must be the live [`Seam`] allocation the subclass was
    /// installed with; [`WindowCommands::attach`] keeps it alive until the
    /// subclass is removed.
    unsafe extern "system" fn callback(
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        id: usize,
        data: usize,
    ) -> isize {
        // SAFETY: `data` is the live `Seam` allocation of the subclass.
        let seam = unsafe { &*(data as *const Seam) };
        if message == WM_NCDESTROY {
            // The window is going away, so the subclass must not outlive it.
            // SAFETY: the OS destroys this HWND on its owner thread.
            unsafe { RemoveWindowSubclass(hwnd, callback, id) };
            seam.attached.store(false, Ordering::SeqCst);
        }
        if let Some(command) = command_from_message(message, wparam) {
            seam.push(command);
            // `WM_COMMAND` has no reply contract (S10 §4 / §9).
            return 0;
        }
        // SAFETY: the original parameters are forwarded along the chain.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// The `WM_COMMAND` seam of one classic window.
    ///
    /// It owns the window — so the handle stays valid while the subclass is
    /// installed — and the shared state the callback writes into.
    pub(crate) struct WindowCommands {
        /// The window the subclass is installed on.
        _window: Arc<Window>,
        hwnd: *mut c_void,
        seam: Arc<Seam>,
    }

    impl WindowCommands {
        /// Installs the seam on `window`.
        pub(crate) fn attach(window: &Arc<Window>) -> std::io::Result<Self> {
            let hwnd = match window.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as *mut c_void,
                _ => return Err(std::io::Error::other("the window has no Win32 handle")),
            };
            let seam = Arc::new(Seam {
                pending: Mutex::new(Vec::new()),
                attached: AtomicBool::new(true),
            });
            let data = Arc::as_ptr(&seam) as usize;
            // SAFETY: the window belongs to this thread, and the `Seam`
            // allocation outlives the subclass (it is dropped only after the
            // removal).
            if unsafe { SetWindowSubclass(hwnd, callback, SUBCLASS_ID, data) } == 0 {
                return Err(std::io::Error::other(
                    "could not install the window-command subclass",
                ));
            }
            Ok(Self {
                _window: Arc::clone(window),
                hwnd,
                seam,
            })
        }

        /// Takes the commands the operating system posted, in order.
        pub(crate) fn take(&self) -> Vec<WindowCommand> {
            self.seam.take()
        }
    }

    impl Drop for WindowCommands {
        fn drop(&mut self) {
            if !self.seam.attached.load(Ordering::SeqCst) {
                return;
            }
            // SAFETY: the window is still alive (the seam holds it) and the
            // `Seam` allocation outlives the callback.
            if unsafe { RemoveWindowSubclass(self.hwnd, callback, SUBCLASS_ID) } == 0 {
                // A failed removal must never leave a dangling callback data
                // pointer. Retain the allocation rather than risk a UAF.
                std::mem::forget(self.seam.clone());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rustamp_ui::ShortcutAction;

        /// The `WM_COMMAND` id sits in the low word of `wparam`; the high word
        /// is the notification code, which these two ids do not use (S03 §1).
        #[test]
        fn the_command_id_is_the_low_word_of_the_message() {
            for (action, command) in [
                (
                    ShortcutAction::ToggleWindowShade,
                    WindowCommand::ToggleWindowShade,
                ),
                (
                    ShortcutAction::ToggleDoubleSize,
                    WindowCommand::ToggleDoubleSize,
                ),
            ] {
                let id = action.command_id() as usize;
                assert_eq!(command_from_message(WM_COMMAND, id), Some(command));
                assert_eq!(
                    command_from_message(WM_COMMAND, (0x1234 << 16) | id),
                    Some(command),
                    "the notification code in the high word must not matter"
                );
            }
        }

        /// Every other message is none of this seam's business: it belongs
        /// to `winit` or to the window procedure below it.
        #[test]
        fn other_messages_carry_no_command() {
            for message in [0x0000, 0x0002, 0x0010, 0x0082, 0x0112] {
                assert_eq!(
                    command_from_message(message, 40064),
                    None,
                    "message {message:#06x} carries no window command"
                );
            }
        }

        /// The commands wait for the presenter and keep their order: the
        /// first one posted is the first one applied.
        #[test]
        fn the_queued_commands_keep_their_order() {
            let seam = Seam {
                pending: Mutex::new(Vec::new()),
                attached: AtomicBool::new(true),
            };
            seam.push(WindowCommand::ToggleWindowShade);
            seam.push(WindowCommand::ToggleDoubleSize);
            assert_eq!(
                seam.take(),
                vec![
                    WindowCommand::ToggleWindowShade,
                    WindowCommand::ToggleDoubleSize
                ]
            );
            assert!(
                seam.take().is_empty(),
                "the queue is empty once it was taken"
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/window_probe.rs"]
mod window_probe;

#[cfg(test)]
mod tests {
    /// The three classic windows carry the ABI classes of S09 §1 (S01 §1 names
    /// them); the dialog does not. Pinned here because a window's class cannot
    /// be read back without opening one, and the capture harness finds the
    /// equalizer and playlist by exactly these strings.
    #[test]
    fn the_classic_windows_carry_their_abi_classes() {
        assert_eq!(
            super::window_class(super::PresenterWindow::Main),
            Some("Winamp v1.x")
        );
        assert_eq!(
            super::window_class(super::PresenterWindow::Equalizer),
            Some("Winamp EQ")
        );
        assert_eq!(
            super::window_class(super::PresenterWindow::Playlist),
            Some("Winamp PE")
        );
        assert_eq!(super::window_class(super::PresenterWindow::Dialog), None);
    }

    use super::*;
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;
    use rustamp_core::config::{Config, WindowLayout};
    use rustamp_core::playlist::MediaLocation;
    use rustamp_core::state::TransportState;
    use rustamp_platform::{DisplaySnapshot, Monitor, ScreenRect, TrayMenuEntry};
    use rustamp_ui::title_bar_inside;

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

    #[test]
    fn tab_belongs_to_the_dialog_and_the_other_shortcuts_stay_global() {
        // The edit-entry box moves its focus with `Tab` and `Shift`+`Tab`,
        // so both chords belong to the dialog instead of the global path.
        assert!(dialog_owns_key(Key::Tab, Modifiers::NONE));
        assert!(dialog_owns_key(Key::Tab, Modifiers::SHIFT));
        // `Ctrl`+`Tab` is the "switch to the next window" row (S03 §1 id
        // 40063) and stays global.
        assert!(!dialog_owns_key(Key::Tab, Modifiers::CTRL));
        assert!(!dialog_owns_key(
            Key::Tab,
            Modifiers::CTRL | Modifiers::SHIFT
        ));
        assert!(!dialog_owns_key(Key::Tab, Modifiers::ALT));
        // Every other key is the dialog's or the global path's, but never
        // the focus key.
        assert!(!dialog_owns_key(Key::Letter(b'E'), Modifiers::CTRL));
        assert!(!dialog_owns_key(Key::Enter, Modifiers::NONE));
    }

    /// No event loop, window, hotkey registration, host process or audio device.
    ///
    /// No display set: the docking tests need no screen clamp, and the
    /// presenter's placement is not what they measure.
    fn desktop_test_presenter(with_tracks: bool) -> Windowed {
        let mut app = App::new(AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
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
        Windowed::new(app, None)
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
        let saved =
            crate::config::serialize_honest(&presenter.app.current_config(), None, None).unwrap();
        let config = crate::config::parse(&saved).unwrap();
        let restored_app = App::new(AppOptions {
            config,
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
            seconds: None,
        })
        .unwrap();
        let mut restored = Windowed::new(restored_app, None);
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

    /// One monitor: the classic 1920x1080 desktop with a 40 px taskbar at the
    /// bottom, so the work area is 1920x1040.
    fn single_monitor() -> DisplaySnapshot {
        DisplaySnapshot::new(vec![Monitor {
            id: "\\\\.\\DISPLAY1".to_string(),
            bounds: ScreenRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work_area: ScreenRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            is_primary: true,
            dpi: 96,
        }])
    }

    /// A saved window position.
    fn saved(id: CoreWindowId, x: i32, y: i32) -> WindowLayout {
        WindowLayout {
            id,
            x,
            y,
            visible: true,
            shaded: false,
            width: None,
            height: None,
        }
    }

    /// An app whose configuration carries the given saved window positions.
    fn app_with_saved_windows(windows: Vec<WindowLayout>) -> App {
        let mut config = Config::default();
        config.ui.windows = windows;
        App::new(AppOptions {
            config,
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
            seconds: None,
        })
        .unwrap()
    }

    /// A saved position that is still on a screen stays exactly where it was:
    /// the player must not move a window the user placed on every start.
    #[test]
    fn a_visible_saved_position_is_not_moved() {
        let app = app_with_saved_windows(vec![
            saved(CoreWindowId::Main, 100, 100),
            saved(CoreWindowId::Equalizer, 100, 216),
            saved(CoreWindowId::Playlist, 100, 332),
        ]);
        let stack = Windowed::restore_stack(&app, Some(&single_monitor()));
        assert_eq!((stack.main.x, stack.main.y), (100, 100));
        assert_eq!((stack.equalizer.x, stack.equalizer.y), (100, 216));
        assert_eq!((stack.playlist.x, stack.playlist.y), (100, 332));
    }

    /// A window that points at a monitor that is gone comes back where its
    /// title strip can be reached: the saved position was far to the left of
    /// the desktop and above its top edge.
    #[test]
    fn a_saved_position_on_a_vanished_monitor_is_pulled_back() {
        let app = app_with_saved_windows(vec![saved(CoreWindowId::Main, -2000, -2000)]);
        let snapshot = single_monitor();
        let stack = Windowed::restore_stack(&app, Some(&snapshot));
        assert!(
            snapshot
                .monitors()
                .iter()
                .any(|monitor| title_bar_inside(&stack.main, &monitor.work_area)),
            "the title strip of {:?} is on no screen",
            stack.main
        );
    }

    /// The playlist's title strip is 20 px high, the other windows' 14 px: a
    /// playlist whose 14 px strip would still fit is pulled in, while the
    /// main window at the same height keeps its place.
    #[test]
    fn the_playlist_uses_its_own_title_strip() {
        let app = app_with_saved_windows(vec![
            saved(CoreWindowId::Main, 100, 1024),
            saved(CoreWindowId::Equalizer, 100, 400),
            saved(CoreWindowId::Playlist, 100, 1024),
        ]);
        let stack = Windowed::restore_stack(&app, Some(&single_monitor()));
        assert_eq!(
            stack.main.y, 1024,
            "the 14 px strip fits, so the main window stays"
        );
        assert_eq!(
            stack.playlist.y, 1020,
            "the 20 px strip does not fit, so the playlist moves up"
        );
    }

    /// A group that pointed at a monitor that is gone comes back in one
    /// piece: the equalizer hangs on the main window and the playlist on the
    /// equalizer, instead of all three landing on the same pixel.
    #[test]
    fn a_vanished_monitor_brings_the_docked_group_back_in_one_piece() {
        let app = app_with_saved_windows(vec![
            saved(CoreWindowId::Main, -2000, -2000),
            saved(CoreWindowId::Equalizer, -2000, -1884),
            saved(CoreWindowId::Playlist, -2000, -1768),
        ]);
        let stack = Windowed::restore_stack(&app, Some(&single_monitor()));
        assert_eq!(
            stack.equalizer.y,
            stack.main.bottom(),
            "the equalizer hangs on the main window"
        );
        assert_eq!(
            stack.playlist.y,
            stack.equalizer.bottom(),
            "the playlist hangs on the equalizer"
        );
    }

    /// Without a screen there is nothing to place a window on: the saved
    /// position is kept and no monitor is invented.
    #[test]
    fn a_snapshot_without_a_monitor_leaves_the_position_alone() {
        let app = app_with_saved_windows(vec![saved(CoreWindowId::Main, -2000, -2000)]);
        let empty = DisplaySnapshot::new(Vec::new());
        let stack = Windowed::restore_stack(&app, Some(&empty));
        assert_eq!((stack.main.x, stack.main.y), (-2000, -2000));
        // And without a display set at all: the enumeration failed.
        let stack = Windowed::restore_stack(&app, None);
        assert_eq!((stack.main.x, stack.main.y), (-2000, -2000));
    }

    /// Exercises actual Moved events and OS readback through the production
    /// presenter, not a lookalike Probe window. No input is synthesized: this
    /// pins Rustamp's explicit programmatic-move snap policy. Real pointer
    /// tracking during winit's modal drag still needs interactive VM acceptance.
    #[test]
    #[ignore = "VM-only: opens the real three-window presenter; run in the guest interactive session"]
    fn docking_presenter_moves_snaps_detaches_reflows_and_restores() {
        require_test_vm();
        if window_probe::isolated(
            "window::tests::docking_presenter_moves_snaps_detaches_reflows_and_restores",
        ) {
            return;
        }
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
                        let text = crate::config::serialize_honest(
                            &self.presenter.app.current_config(),
                            None,
                            None,
                        )
                        .unwrap();
                        let app = App::new(AppOptions {
                            config: crate::config::parse(&text).unwrap(),
                            skin: crate::skin::builtin(),
                            output: OutputChoice::Null,
                            volume: 200,
                            media: None,
                            seconds: None,
                        })
                        .unwrap();
                        self.presenter = Windowed::new(app, None);
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

    /// The playlist editor's windowshade, end to end through the production
    /// presenter: the skinned shade button flips the state, the OS window's
    /// client area becomes the 14 px shade band (S01 §1) and the editor keeps
    /// the place the docked stack gave it (S05 §3.3).
    ///
    /// The click goes through the app's own handler - the same entry point
    /// the OS click takes - so no pointer is synthesized and no window is
    /// driven from outside.
    ///
    /// VM-only: it opens the real windows.
    #[test]
    #[ignore = "VM-only: opens the real presenter; run in the guest interactive session"]
    fn the_playlist_window_shade_resizes_the_real_window() {
        require_test_vm();
        if window_probe::isolated(
            "window::tests::the_playlist_window_shade_resizes_the_real_window",
        ) {
            return;
        }
        struct Probe {
            presenter: Windowed,
            phase: u8,
            next: Instant,
            deadline: Instant,
            /// The editor's outer origin before the shade, to show that the
            /// docked place does not move.
            origin: Option<(i32, i32)>,
        }
        impl Probe {
            fn client_size(&self) -> (u32, u32) {
                let window = self
                    .presenter
                    .playlist_window
                    .as_ref()
                    .expect("the playlist window is open");
                let size = window.inner_size();
                assert_eq!(
                    window.outer_size(),
                    size,
                    "the shade must remain borderless"
                );
                (size.width, size.height)
            }
            fn origin(&self) -> (i32, i32) {
                let position = self
                    .presenter
                    .playlist_window
                    .as_ref()
                    .expect("the playlist window is open")
                    .outer_position()
                    .expect("the playlist window has a position");
                (position.x, position.y)
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
                assert!(
                    Instant::now() < self.deadline,
                    "the playlist shade probe timed out"
                );
                self.presenter.about_to_wait(event_loop);
                if Instant::now() < self.next {
                    return;
                }
                match self.phase {
                    0 => {
                        // At rest the editor is the spec'd 275 x 116.
                        assert_eq!(self.client_size(), (275, 116));
                        self.origin = Some(self.origin());
                        // The skinned shade button (S01 §4: 9 x 9 at (254,3)).
                        self.presenter
                            .app
                            .playlist_mouse_down(254, 3, Modifiers::NONE)
                            .expect("the shade click applies");
                    }
                    // The resize is requested in the tick after the click, so
                    // the size is read one phase later.
                    1 => {}
                    2 => {
                        assert_eq!(self.client_size(), (275, 14), "one row tall");
                        assert_eq!(
                            Some(self.origin()),
                            self.origin,
                            "the editor keeps its docked place"
                        );
                        self.presenter
                            .app
                            .playlist_mouse_down(254, 3, Modifiers::NONE)
                            .expect("the shade click applies");
                    }
                    3 => {}
                    4 => {
                        assert_eq!(self.client_size(), (275, 116), "back to the rest size");
                        assert_eq!(
                            Some(self.origin()),
                            self.origin,
                            "the editor keeps its docked place"
                        );
                        self.presenter.app.set_playlist_size(550, 348);
                    }
                    5 => {}
                    6 => {
                        assert_eq!(self.client_size(), (550, 348));
                        let image = self.presenter.app.render_playlist();
                        assert_eq!((image.width(), image.height()), self.client_size());
                        self.presenter.paint_playlist();
                        self.presenter.app.set_playlist_size(275, 116);
                    }
                    7 => {}
                    8 => {
                        assert_eq!(self.client_size(), (275, 116));
                        self.presenter
                            .app
                            .submit(Command::ToggleDoubleSize)
                            .unwrap();
                    }
                    9 => {}
                    10 => {
                        assert_eq!(self.client_size(), (550, 232));
                        let image = self.presenter.app.render_playlist();
                        assert_eq!((image.width(), image.height()), self.client_size());
                        self.presenter.paint_playlist();
                        event_loop.exit();
                        return;
                    }
                    _ => unreachable!(),
                }
                self.phase += 1;
                self.next = Instant::now() + Duration::from_millis(150);
            }
        }
        let mut probe = Probe {
            presenter: desktop_test_presenter(false),
            phase: 0,
            next: Instant::now() + Duration::from_millis(150),
            deadline: Instant::now() + Duration::from_secs(10),
            origin: None,
        };
        build_event_loop(true).unwrap().run_app(&mut probe).unwrap();
        assert_eq!(probe.phase, 10);
    }

    /// Keep the runner's inventory name, but measure the system popup rather
    /// than the retired skin-painted 323 x 569 client window (S04 §1).
    #[cfg(windows)]
    #[test]
    #[ignore = "VM-only: opens and measures the production native main menu"]
    fn the_menu_popup_opens_at_the_measured_size() {
        require_test_vm();
        if window_probe::isolated("window::tests::the_menu_popup_opens_at_the_measured_size") {
            return;
        }
        struct Probe {
            presenter: Windowed,
            measured: bool,
        }
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.presenter.resumed(event_loop);
                let origin = self
                    .presenter
                    .window
                    .as_ref()
                    .unwrap()
                    .outer_position()
                    .unwrap();
                let menu = self.presenter.app.native_main_menu();
                let observer = window_probe::observe_popup(
                    self.presenter.raw_main_window(),
                    menu_popup_origin((origin.x, origin.y)),
                    menu.rows,
                );
                self.presenter.app.submit(Command::OpenMainMenu).unwrap();
                self.presenter.drain_main_menu();
                observer.join().expect("popup observer panicked").unwrap();
                assert!(!self.presenter.app.main_menu_is_open());
                assert!(!self.presenter.app.quit(), "dismissal must not choose Exit");
                self.measured = true;
                event_loop.exit();
            }
            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let mut probe = Probe {
            presenter: desktop_test_presenter(false),
            measured: false,
        };
        build_event_loop(true).unwrap().run_app(&mut probe).unwrap();
        assert!(
            probe.measured,
            "the real presenter must show its native popup"
        );
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
                presenter.app.playlist_frame_size(),
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
        assert_eq!(presenter.app.playlist_frame_size(), (275, 116));
        assert_eq!(rustamp_ui::PLAYLIST_WINDOW_SIZE, (275, 116));
    }

    /// The ADD defect: the button opened a window of ours that asked for a
    /// typed path or a drop onto it, instead of the standard Windows
    /// file-open dialog.
    ///
    /// The routing decision is checked here: the files face is the file-open
    /// dialog and the directory face is the shell's folder dialog - a folder
    /// is not a file, so the file-open dialog cannot ask for one. The
    /// requesting command's `play` flag rides along in the dialog kind, not
    /// in the route. Multiple selection and the decoding of what the user
    /// picked are checked in `rustamp-platform`
    /// (`file_dialog::parse_selection`, `file_dialog::directory_result`).
    /// That the OS then shows the dialog needs a desktop, so only the
    /// owner's machine can confirm it.
    #[test]
    fn the_files_and_the_directory_face_of_open_media_are_native() {
        for play in [true, false] {
            assert_eq!(
                DialogRoute::for_dialog(DialogKind::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play,
                }),
                DialogRoute::NativeFilePicker,
                "the files face is the standard file-open dialog"
            );
            assert_eq!(
                DialogRoute::for_dialog(DialogKind::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play,
                }),
                DialogRoute::NativeDirectoryPicker,
                "the directory face is the shell's folder dialog"
            );
        }
        for kind in [
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            },
            DialogKind::About,
            DialogKind::FileInfo,
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
    fn a_pending_media_dialog_is_drained_exactly_once() {
        for kind in [OpenMediaKind::Files, OpenMediaKind::Directory] {
            let mut presenter = desktop_test_presenter(false);
            presenter.pending_media_pick = Some(DialogKind::OpenMedia { kind, play: false });
            // No main window, so there is no owner handle and the platform call
            // refuses before any dialog exists; the request is still consumed.
            presenter.drain_media_pick();
            assert!(presenter.pending_media_pick.is_none());
            presenter.drain_media_pick();
            assert!(presenter.pending_media_pick.is_none());
            assert!(presenter.window.is_none(), "the test must stay headless");
        }
    }

    /// The app's own dialog gives way to the native one.
    ///
    /// The app stays headless: it only *requests* a media choice, and the
    /// presenter decides how the OS asks. The request is recorded and
    /// drained from the tick, because a modal dialog must not be opened from
    /// inside a window procedure.
    #[test]
    fn the_directory_face_is_taken_over_by_the_native_folder_dialog() {
        let dialog = DialogKind::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: true,
        };
        let mut presenter = desktop_test_presenter(false);
        presenter
            .app
            .submit(Command::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            })
            .unwrap();
        assert!(
            presenter.app.dialog_is_open(dialog),
            "the app requests the choice"
        );
        assert!(presenter.route_open_dialog(dialog));
        assert!(
            !presenter.app.dialog_is_open(dialog),
            "the app's own dialog gives way to the native one"
        );
        assert_eq!(
            presenter.pending_media_pick,
            Some(dialog),
            "the request waits for the tick"
        );
    }

    /// A chosen directory contributes the media files it holds.
    ///
    /// The chosen directory itself is not an entry: the pick brings in the
    /// media below it (the depth is Rustamp's own choice, see
    /// [`crate::directory`]). The files that are not media are left out.
    #[test]
    fn a_chosen_directory_adds_its_media_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp3"), b"").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        std::fs::create_dir_all(dir.path().join("album")).unwrap();
        std::fs::write(dir.path().join("album/b.flac"), b"").unwrap();

        let mut presenter = desktop_test_presenter(false);
        presenter.apply_media_pick(MediaPick::Directory(Some(dir.path().to_path_buf())), false);
        assert_eq!(
            presenter.app.playlist().len(),
            2,
            "the two media files of the tree are added, the text file is not"
        );
    }

    /// A cancelled dialog adds nothing.
    #[test]
    fn a_cancelled_media_pick_adds_nothing() {
        let mut presenter = desktop_test_presenter(false);
        presenter.apply_media_pick(MediaPick::Directory(None), true);
        presenter.apply_media_pick(MediaPick::Files(Vec::new()), true);
        assert_eq!(presenter.app.playlist().len(), 0);
    }

    /// A folder without a media file says so instead of doing nothing.
    #[test]
    fn a_directory_without_media_files_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        let mut presenter = desktop_test_presenter(false);
        presenter.apply_media_pick(MediaPick::Directory(Some(dir.path().to_path_buf())), false);
        assert_eq!(presenter.app.playlist().len(), 0);
        let notes = presenter.app.drain_notes();
        assert_eq!(
            notes.len(),
            1,
            "the player says that nothing was added: {notes:?}"
        );
        assert!(notes[0].contains("no media file"), "{notes:?}");
    }

    /// The windowed path prints a note **once**.
    ///
    /// The presenter's rule is the app's rule ([`App::print_notes`]): the
    /// queue is drained, so a second print has nothing left to print. A note
    /// the tick printed is never printed again.
    #[test]
    fn the_presenter_prints_a_note_once() {
        let mut presenter = desktop_test_presenter(false);
        presenter.app.note("a note for the user".to_string());
        presenter.print_notes();
        // The tick printed the note: the queue is empty, and a second print
        // finds nothing to print.
        assert!(presenter.app.drain_notes().is_empty());
        presenter.print_notes();
        assert!(presenter.app.drain_notes().is_empty());
    }

    /// The tick is the only place the windowed run prints the notes.
    ///
    /// A second printing site in the tick would print a note twice.
    #[test]
    fn the_tick_prints_the_notes_once() {
        let source = include_str!("window.rs");
        let tick = source
            .split("fn about_to_wait(")
            .nth(1)
            .unwrap()
            .split("fn exiting(")
            .next()
            .unwrap();
        assert_eq!(
            tick.matches("print_notes(").count(),
            1,
            "the tick prints the notes once: {tick}"
        );
    }

    /// A failed dialog adds nothing.
    #[test]
    fn a_failed_media_pick_adds_nothing() {
        let mut presenter = desktop_test_presenter(false);
        presenter.apply_media_pick(
            MediaPick::DirectoryFailed(rustamp_platform::FileDialogError::NullOwner),
            true,
        );
        presenter.apply_media_pick(
            MediaPick::FileFailed(rustamp_platform::FileDialogError::NullOwner),
            true,
        );
        assert_eq!(presenter.app.playlist().len(), 0);
    }

    /// The chosen files are **appended** to the playlist, not replacing it.
    ///
    /// The `play` flag of the request is pinned by the command itself (see
    /// [`crate::directory::add_command`]); here the playlist is the
    /// observable.
    #[test]
    fn the_chosen_files_are_appended_to_the_playlist() {
        let mut presenter = desktop_test_presenter(true);
        let before = presenter.app.playlist().len();
        presenter.add_media_paths(vec![PathBuf::from("C:/music/a.mp3")], false);
        assert_eq!(
            presenter.app.playlist().len(),
            before + 1,
            "the chosen file is appended to the entries that are already there"
        );
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

    /// The paint gate: a tick with nothing new draws nothing.
    ///
    /// What a headless test can pin is the mask the presenter paints from:
    /// the tick still runs (the app's pump is unchanged), but no window is
    /// marked, so `present_frames` draws no frame. That the blit itself is
    /// skipped only a profile can show.
    #[test]
    fn a_tick_with_nothing_new_has_no_frame_to_present() {
        let mut presenter = desktop_test_presenter(false);
        // The first frame is new to every window.
        assert!(
            presenter.dirty.main,
            "a fresh presenter paints its first frame"
        );
        presenter.dirty.merge(presenter.app.take_window_dirty());
        presenter.present_frames();
        assert_eq!(presenter.dirty, WindowDirty::NONE);
        // Nothing at all happens: no track, stopped, no input, no scan.
        presenter.app.pump().unwrap();
        presenter.dirty.merge(presenter.app.take_window_dirty());
        assert_eq!(
            presenter.dirty,
            WindowDirty::NONE,
            "an idle tick has no frame to draw"
        );
    }

    /// The clock still marks frames: the analyser ages from the clock alone,
    /// in the same tick cadence as before the gate.
    #[test]
    fn the_clock_still_marks_the_main_window() {
        use rustamp_core::event::Event;
        use rustamp_core::vis::{VIS_SAMPLES, VisFrame};

        let mut presenter = desktop_test_presenter(false);
        presenter
            .app
            .ui_mut()
            .handle_event(&Event::StateChanged(TransportState::Playing), 0);
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
            .push_vis_frame(&frame, 32_000, 0)
            .expect("the analyser accepts a finite frame");
        // The first report is discarded: the frame itself is the baseline.
        let _ = presenter.app.take_window_dirty();
        // Time passes with no input at all.
        presenter.app.ui_mut().refresh(400);
        assert!(
            presenter.app.take_window_dirty().main,
            "the aged analyser is a changed main-window frame"
        );
    }

    /// A command is the user's own action and may change any frame, so it
    /// marks every window rather than deciding which one it reaches.
    #[test]
    fn a_command_marks_the_windows_it_can_change() {
        let mut presenter = desktop_test_presenter(false);
        let _ = presenter.app.take_window_dirty();
        presenter.app.submit(Command::SetVolume(40)).unwrap();
        assert_eq!(
            presenter.app.take_window_dirty(),
            WindowDirty::ALL,
            "a command marks every window it could have changed"
        );
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

    /// The focus defect: opening the equalizer or the playlist editor took
    /// the keyboard focus away from the main window, so the main window drew
    /// its **unfocused** title bar while Winamp keeps the focused one. The
    /// measured reference start state of the base-2.91 campaign is exactly
    /// that - "main focused, equalizer and playlist open" - and our capture
    /// of it showed the unfocused band (2026-10-21).
    ///
    /// What this can and cannot do: the decision is checked here, headlessly.
    /// That the shell then shows the window without activating it needs a
    /// desktop, so only the owner machine can confirm the rest.
    #[test]
    fn the_classic_sibling_windows_leave_the_focus_with_the_main_window() {
        for window in [PresenterWindow::Equalizer, PresenterWindow::Playlist] {
            let attributes = Windowed::window_attributes(window, "Rustamp", (275, 116), None);
            assert!(
                !attributes.active,
                "{window:?}: opening it must leave the keyboard focus with the main window"
            );
        }
        for window in [PresenterWindow::Main, PresenterWindow::Dialog] {
            let attributes = Windowed::window_attributes(window, "Rustamp", (275, 116), None);
            assert!(
                attributes.active,
                "{window:?}: this window takes the focus when it opens"
            );
        }
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
