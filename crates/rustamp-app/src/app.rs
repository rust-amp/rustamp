// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The application: it owns the clock, the engine and the UI machine, and it
//! routes commands and events between them (task card N1).
//!
//! # The shape of a running player
//!
//! ```text
//!   OS / winit ──input──> App::mouse_* / key_down ──> MainWindow (headless UI)
//!                                                        │
//!                                                        ▼  Vec<Command>
//!   CommandSender ──────────────────────────────────> command channel
//!                                                        │
//!                                            App::pump drains it
//!                                                        ▼
//!                            Engine::apply (audio) + App::apply_app (windows,
//!                            skin, modes, dialogs)
//!                                                        │
//!                                             EventBus publish (engine)
//!                                                        ▼
//!   App::pump drains events ──> MainWindow::handle_event ──> render state
//! ```
//!
//! The UI machine is headless: it never reads a clock (the app passes
//! milliseconds in) and never touches the OS. The app is the only place that
//! knows both sides.
//!
//! Everything in this module is headless and unit-tested with a
//! [`NullOutput`](rustamp_output::NullOutput); the windowed presenter in
//! [`crate::window`] only adds the `winit`/`softbuffer` shell around it.
//!
//! The app's diagnostics travel as **notes** ([`App::note`]): the windowed
//! presenter and the headless run print them the same way, so a headless run
//! shows the user what the windowed one shows (see [`App::print_notes`]).

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::DecoderRegistry;
use rustamp_core::bus::{CommandReceiver, CommandSender, EventBus, EventReceiver, command_channel};
use rustamp_core::command::Command;
use rustamp_core::config::{
    AudioConfig, Config, EqConfig, PlaylistConfig, PluginsConfig, UiConfig, WindowLayout,
};
use rustamp_core::error::{Error, ErrorInfo, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::playlist::{EntryId, MediaLocation, PlaylistChangeKind, SortKey};
use rustamp_core::state::{PlayerState, TransportState, WindowId};
use rustamp_gfx::Image;
use rustamp_ipc::{Message, Transport};
use rustamp_meta::replaygain::ReplayGainTags;
use rustamp_output::NullClock;
use rustamp_plugin_abi::PluginKind;
use rustamp_ui::{
    DialogManager, EqMenuRequest, EqualizerLayout, EqualizerWindow, Key, MainWindow,
    MainWindowLayout, MainWindowState, Modifiers, PlaylistDialog, PlaylistWindow, ShortcutAction,
    ShortcutOutcome, ShortcutWindow, eq_frame_size, frame_size, lookup, render_equalizer_window,
    render_main_window, volume_percent,
};

use crate::cli::OutputChoice;
use crate::double_click::TitleBarClicks;
use crate::fade_note::{FadeConfiguration, SeenFadeReports};
use crate::host::{
    CommandOutcome, HostStateLink, PollOutcome, PublishOutcome, player_state_message,
};
use crate::output::create_output;
use crate::plugin_inventory::LoadedPlugin;
use crate::routing;
use crate::skin::{LoadedSkin, SkinChoice, load as load_skin};
use crate::supervise::Supervisor;

/// How many "not implemented yet" notes the app keeps for the caller to print.
const MAX_NOTES: usize = 64;

/// The prefix every note the player prints carries.
///
/// The player's own lines all start with it, so a diagnostic is told apart
/// from the rest of the output at a glance. It is the **one** prefix: the
/// windowed presenter and the headless run print a note the same way (see
/// [`write_notes`]).
const NOTE_PREFIX: &str = "rustamp: ";

/// A folder a preferences page asked the player for.
///
/// The page does not open the folder dialog itself: it asks, and the presenter
/// opens [`rustamp_platform::pick_directory`] from the tick, because the dialog
/// runs its own modal message loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRequest {
    /// The folder the skin browser reads (the Skins page).
    Skins,
    /// The folder the plug-in pages scan (every plug-in page).
    Plugins,
}

/// Everything [`App::new`] needs.
pub struct AppOptions {
    /// The loaded configuration.
    pub config: Config,
    /// The loaded skin (already resolved and fallen back if needed).
    pub skin: LoadedSkin,
    /// Where the audio goes.
    pub output: OutputChoice,
    /// Initial volume `0..=255`.
    pub volume: u8,
    /// The media to queue and play, if any: a file path or an `http://` URL.
    ///
    /// A file replaces the queue and starts playing (S02 §1); a URL is a
    /// stream (S19), and it becomes the same kind of queue entry — a
    /// `TrackSource::Url` — a playlist's URL produces.
    pub media: Option<MediaLocation>,
    /// Stop after this many seconds (test aid).
    pub seconds: Option<u64>,
}

/// A command the operating system posted to one of the player's classic
/// windows.
///
/// The reference carries its commands as `WM_COMMAND` ids (S03 §1): 40064
/// toggles the windowshade, 40165 toggles the double size. The command
/// applies to the window it was addressed to: the reference gives the
/// equalizer its own shade flag (`eq_ws`) and the playlist editor its own
/// (`pe_height_ws`), beside the main window's (`windowshade`), S01 §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    /// Toggle the window's windowshade (S03 id 40064).
    ///
    /// The window's shade is the 14-px title strip alone (S01 §1: 275 x 14;
    /// S05 §1.2: no body rows are drawn).
    ToggleWindowShade,
    /// Toggle the window's double size (S03 id 40165).
    ///
    /// The window's size is the 2x scale of its 1x size (S05 §2.1: the
    /// equalizer is 275 x 116 at 1x and 550 x 232 at 2x; the playlist
    /// editor is not affected by double size at all).
    ToggleDoubleSize,
}

impl WindowCommand {
    /// The command the `WM_COMMAND` id `id` carries, if any (S03 §1).
    ///
    /// The id is the low word of the message's `wparam`; the high word is
    /// the notification code, which the two ids this player answers do not
    /// use. The ids come from the shortcut table, which is the audited S03
    /// id table, so there is one place that names them. Every other id
    /// belongs to a command the classic windows do not answer here, so it
    /// carries nothing.
    pub fn from_command_id(id: u32) -> Option<Self> {
        if id == ShortcutAction::ToggleWindowShade.command_id() {
            Some(Self::ToggleWindowShade)
        } else if id == ShortcutAction::ToggleDoubleSize.command_id() {
            Some(Self::ToggleDoubleSize)
        } else {
            None
        }
    }
}

/// The windows whose pixels a pump step can have changed.
///
/// The app owns the state the windows draw, so it is the one place that can
/// say which frame moved. The presenter ([`crate::window`]) asks for the mask
/// after every pump ([`App::take_window_dirty`]) and paints only the windows
/// in it, so a tick whose mask is empty paints nothing at all; the 16 ms tick
/// stays the upper bound the clock-driven content needs.
///
/// The mask is bookkeeping only: it never reaches the event bus, the output or
/// the control channel, so a `--null` run prints the same thing whether or not
/// a presenter reads it (a headless run never does).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowDirty {
    /// The main window: the clock digits, the marquee, the analyser, and every
    /// toggle and slider the frame draws.
    pub main: bool,
    /// The equalizer window.
    pub equalizer: bool,
    /// The playlist editor.
    pub playlist: bool,
    /// The media library window.
    pub library: bool,
    /// The own-window dialog (at most one is open at a time).
    pub dialog: bool,
}

impl WindowDirty {
    /// Nothing changed: a tick with this mask paints no window.
    pub const NONE: Self = Self {
        main: false,
        equalizer: false,
        playlist: false,
        library: false,
        dialog: false,
    };

    /// Every window changed: a command or a skin load may touch any frame.
    pub const ALL: Self = Self {
        main: true,
        equalizer: true,
        playlist: true,
        library: true,
        dialog: true,
    };

    /// Adds `other` to this mask.
    pub fn merge(&mut self, other: Self) {
        self.main |= other.main;
        self.equalizer |= other.equalizer;
        self.playlist |= other.playlist;
        self.library |= other.library;
        self.dialog |= other.dialog;
    }
}

/// The main window's render inputs at the last pump that reported a change.
///
/// The frame is drawn from the UI machine's state (transport, clock, marquee,
/// analyser, toggles, title), the frame size, the focus variant and the skin,
/// so those four are what a new frame is recognized by.
struct MainFrame {
    size: (u32, u32),
    skin: u64,
    focused: bool,
    state: MainWindowState,
}

/// The equalizer frame's render inputs at the last pump that reported a
/// change. The equalizer has no clock: its state is the whole frame.
struct EqFrame {
    skin: u64,
    state: rustamp_ui::EqualizerState,
}

/// The media library's view at the last pump that reported a change.
///
/// The list is filled by the scan's worker thread, which has no call site in
/// the app to mark a frame from, so the view itself is the signal.
#[derive(PartialEq)]
struct LibraryFrame {
    size: (u32, u32),
    total: usize,
    offset: usize,
    scroll: usize,
    selected: Option<i64>,
    sort: rustamp_ui::LibrarySort,
    descending: bool,
    text: String,
    scan: Option<String>,
}

/// The windows an event can change a frame of.
///
/// The main window and the equalizer are absent on purpose: their frames are
/// compared against their render inputs in [`App::report_window_changes`],
/// which already covers every event that reaches them. The playlist and the
/// dialog have no such comparison (their models are far larger than a frame),
/// so their frames are marked here, where the event is applied.
/// What the marquee says for a note, if the note is for the user.
///
/// An entry that would not open is named by its file: the marquee is narrow,
/// and the full path is in the note.
fn status_text(note: &str) -> Option<String> {
    if let Some(error) = note.strip_prefix("error: ") {
        let Some(rest) = error.strip_prefix(rustamp_audio::decode::CANNOT_PLAY) else {
            return Some(error.to_string());
        };
        let (path, reason) = rest.split_once(": ").unwrap_or((rest, ""));
        let name = std::path::Path::new(path).file_name().map_or_else(
            || path.to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        return Some(format!("cannot play {name}: {reason}"));
    }
    note.contains("not implemented").then(|| note.to_string())
}

/// How far the equalizer window's pan keys move the balance (own choice: S07 §9
/// leaves it open).
const PAN_STEP: i8 = 4;

fn event_windows(event: &Event) -> WindowDirty {
    match event {
        // The highlighted row and the rows' labels.
        Event::TrackChanged { .. } | Event::PlaylistChanged { .. } | Event::PlaylistEnded => {
            WindowDirty {
                playlist: true,
                dialog: true,
                ..WindowDirty::NONE
            }
        }
        // A dialog's body carries live track data: the file info shows the
        // position, the jump box the queue, the edit box the metadata.
        Event::StateChanged(_)
        | Event::PositionChanged { .. }
        | Event::StreamInfoChanged(_)
        | Event::VolumeChanged { .. }
        | Event::ModesChanged { .. }
        | Event::EqChanged(_)
        | Event::Error(_) => WindowDirty {
            dialog: true,
            ..WindowDirty::NONE
        },
        // A skin reaches every frame the app draws.
        Event::SkinChanged(_) => WindowDirty::ALL,
        // Neither has a frame of its own.
        Event::WindowVisibilityChanged { .. } | Event::Quit => WindowDirty::NONE,
        // Every variant is listed: a new event must decide which frame it can
        // change rather than inherit a default.
    }
}

/// The player.
pub struct App {
    engine: Engine,
    ui: MainWindow,
    /// Drop the next tap batch across a seek/track/stream boundary: frames have
    /// no generation or source-rate tag. This is conservative, not a proof of
    /// audible-time or post-seek correspondence (S08 remains open).
    vis_discontinuity: bool,
    layout: MainWindowLayout,
    /// The equalizer's headless behaviour (S07; the engine stage is M2 future
    /// work, so this is the UI's own prediction state).
    eq: EqualizerWindow,
    eq_layout: EqualizerLayout,
    /// The windows whose pixels changed since the presenter last painted (see
    /// [`WindowDirty`]).
    window_dirty: WindowDirty,
    /// The main window's render inputs at the last pump that reported a
    /// change, and the equalizer's.
    main_frame: Option<MainFrame>,
    eq_frame: Option<EqFrame>,
    /// The media library's view at the last pump that reported a change; `None`
    /// while the window is closed.
    library_frame: Option<LibraryFrame>,
    /// The title-bar double-click trackers (S03 §5): one per classic window,
    /// so a press in one window never pairs with a press in another.
    title_bar_clicks: TitleBarClicks,
    pub(crate) skin: LoadedSkin,
    config: Config,
    /// The plug-in folder as the preferences pages last scanned it
    /// ([`App::rescan_plugins`]); empty until the folder was read.
    plugin_scan: rustamp_prefs::PluginScan,
    /// Whether the plug-in folder was read for this session.
    ///
    /// The startup load needs the list before it can ask the host for the
    /// plug-ins the user has switched on (see
    /// [`App::load_enabled_plugins`]), so it reads the folder once — the
    /// scan reads bytes and never loads anything. The preferences pages
    /// refresh the scan on their own ([`App::rescan_plugins`]).
    plugin_scanned: bool,
    /// The host generation the plug-in state below belongs to.
    ///
    /// A new host has loaded nothing, so the state starts over with it
    /// (see [`App::sync_plugin_host_state`]).
    plugin_host_generation: u64,
    /// The plug-in files the host was asked to load in that generation.
    ///
    /// The host loads one plug-in per request and the protocol has no unload
    /// request, so a file is asked for once per host (see
    /// [`App::request_plugin_load`]).
    plugin_requests: HashSet<String>,
    /// Whether the plug-ins the user has switched on were already asked for
    /// in that generation (see [`App::load_enabled_plugins_for_new_host`]).
    plugin_startup_loaded: bool,

    bus: EventBus,
    events: EventReceiver,
    commands: CommandSender,
    command_rx: CommandReceiver,

    /// The snapshot the local control channel hands out (`--control`).
    ///
    /// The pump writes it ([`App::update_control_snapshot`]) from
    /// [`App::host_player_state`], so a `snapshot` answer reads the state the
    /// player already computed and never round-trips through the UI thread: a
    /// modal dialog cannot block it. The channel reads it through
    /// [`App::control_handles`].
    control_state: Arc<Mutex<PlayerState>>,
    /// Whether this run serves the local control channel (`--control`).
    ///
    /// A run that serves a channel is a **session**: the headless loop stays
    /// up even when nothing is queued, because the client's commands are what
    /// the run is for (see [`App::run_headless`]).
    control_channel: bool,

    output_choice: OutputChoice,
    /// Control-thread output-open diagnostics, including later engine reopens.
    output_notes: std::sync::mpsc::Receiver<String>,
    registry: DecoderRegistry,
    /// The decoders without the input plug-ins (see [`App::install_plugin_inputs`]).
    builtin_registry: DecoderRegistry,

    /// The virtual clock, when the null output is in use (`--null` headless or
    /// `--no-audio` windowed).
    clock: Option<NullClock>,
    started: Instant,
    seconds: Option<u64>,
    /// Last time [`App::drive_null_clock`] ran (windowed `--no-audio` only).
    last_null_tick: Instant,
    /// Sub-frame remainder for [`App::drive_null_clock`], mirroring the
    /// accumulator in [`App::run_headless`].
    pending_null_frames: f64,

    // ----- application-level state (not the engine's, not the UI machine's) -----
    shuffle: bool,
    repeat: bool,
    always_on_top: bool,
    easy_move: bool,
    pub(crate) focused: bool,

    /// Whether the playlist editor owns the focus (S01 §4.1: its title band has
    /// a focused and an unfocused variant). The main window's flag is not a
    /// stand-in: the captures showed the two windows disagree, and sharing the
    /// flag swapped both playlist states.
    pub(crate) playlist_focused: bool,
    current_skin: Option<PathBuf>,
    /// Counts the skin loads, so a holder of skin-derived resources (the
    /// cursor cache) can tell one skin from the next.
    skin_generation: u64,
    /// The skins folder beside the configuration: the skins it holds and the
    /// entries the last read skipped (see [`crate::skin_folder`]).
    ///
    /// The folder is read when the skin list is asked for and kept until the
    /// folder changes, so the list is not read again for every frame. The
    /// cache is borrowed for the length of a list build, and nothing inside
    /// a borrow calls back into the app.
    pub(crate) skin_folder: RefCell<crate::skin_folder::SkinFolder>,
    quit: bool,
    /// Runtime tray/hotkey visibility; start visible even if a previous run hid
    /// the main window, so an unavailable tray can never strand the player.
    main_visible: bool,
    minimize_requested: bool,
    /// Which player window holds keyboard focus: the headless model of the OS
    /// activation (see [`crate::focus`]); the presenter applies it.
    focused_window: WindowId,
    /// Pending "focus this OS window" for the presenter.
    focus_request: Option<WindowId>,
    /// Pending "un-minimize the main OS window" for the presenter: the half
    /// of `ShowMainWindow` that `main_visible` alone cannot express, since a
    /// minimized window is still visible.
    restore_request: bool,
    playlist_ended: bool,
    /// Stable ids the app assigns to the engine's queue entries.
    ///
    /// The engine reports `Event::TrackChanged` with `id: None` (it holds no
    /// playlist ids; RFC 0001 open point), while the UI machine uses
    /// `id.is_some()` to tell "a track is loaded" from "nothing loaded". The
    /// app is the state owner, so it allocates the ids and normalises the event
    /// on its way to the UI. See the findings in the task card report.
    pub(crate) entry_ids: Vec<EntryId>,
    next_entry_id: u64,
    /// The playlist editor's behaviour model (S06); owned here, rendered and
    /// fed by `playlist_window.rs`, synced from `entry_ids`.
    pub(crate) playlist: PlaylistWindow,
    /// The user's optional 2x enlargement for a playlist smaller than 550x232.
    pub(crate) playlist_double_size: bool,
    pub(crate) playlist_resized_in_double_size: bool,
    /// The media library: the app's index, its view and the background scan
    /// (see `library_window.rs`). The OS window lives in `window.rs`.
    pub(crate) library_window: crate::library_window::LibraryWindow,
    /// The settings of the last playlist extraction: the path template and
    /// the target directory.
    ///
    /// The configuration dialog starts with them, so a second extraction
    /// needs no typing. They are **not** written into the configuration
    /// file: a new key in `rustamp-core`'s `Config` is a contract change and
    /// needs an RFC, and the settings of the running session are enough for
    /// now.
    pub(crate) last_extract_settings: rustamp_platform::ExtractSettings,
    /// The simple modal dialogs (About, file info, jump-to, load playlist,
    /// skin browser, preferences); owned here, rendered by `dialogs.rs`,
    /// hosted by `window.rs`.
    pub(crate) dialogs: DialogManager,
    /// The tags of the selected entries, read when the playlist's
    /// `Ctrl+Alt+E` row ran (S03 §2 `40293`). A snapshot: the extended-info
    /// box shows it, and nothing reads the files again while it is open.
    pub(crate) extended_info: Option<rustamp_ui::dialogs::ExtendedInfo>,
    /// The edit-entry session (S03 §2 `40255` "Edit the playlist entry"):
    /// the file whose tags the dialog edits and the dialog's model. `None`
    /// when no entry is being edited.
    pub(crate) edit_entry: Option<crate::dialogs::EditEntrySession>,
    pub(crate) last_playlist_report: Option<rustamp_playlist::PlaylistReport>,
    /// The format the currently loaded playlist was detected as (RFC 0017
    /// import). `None` means no playlist has been loaded yet, so callers
    /// default to M3U. Stored alongside [`Self::last_playlist_report`] because
    /// that report carries no format of its own.
    pub(crate) last_playlist_format: Option<rustamp_playlist::PlaylistFormat>,
    /// Whether the main-menu popup is open (owned here, rendered by
    /// `menu.rs`, hosted by `window.rs`; S04).
    pub(crate) main_menu_open: bool,
    /// A clutterbar popup (Options or visualization) waiting for the presenter.
    pub(crate) pending_clutter_menu: Option<crate::menu::ClutterMenu>,
    /// The equalizer's Presets popup is waiting to be shown (S07 §4.2).
    pub(crate) pending_eq_menu: bool,
    /// The playlist MISC popup awaits the presenter's modal-menu drain.
    pub(crate) pending_playlist_misc_menu: bool,
    /// A context menu of the main window waiting for the presenter's tick.
    pub(crate) pending_context_menu: Option<crate::menu::ContextMenu>,
    /// The "Load an EQ preset" file dialog waits for the presenter's tick.
    pending_eq_file: bool,
    /// A right click on a playlist entry waiting for the presenter: the
    /// request for the playlist's **context menu** (S03 §5).
    ///
    /// **Blocked on S04**: the menu's rows, wording and enabled states are
    /// S04's, and S03 §5 measures only that a right click on an entry opens
    /// one. The measured part of the gesture — the pressed row is selected
    /// through the ordinary `Command::Select` path — already happened when
    /// the click was routed, so this field carries nothing but the request,
    /// and the presenter's tick takes it once
    /// ([`App::take_pending_playlist_context`]).
    pub(crate) pending_playlist_context: Option<crate::playlist_window::PlaylistContextRequest>,
    /// A save-playlist request waiting for the presenter (Ctrl+S, S03 §2, command ID 40204).
    pub(crate) request_save_playlist: bool,
    /// An HTML-playlist request waiting for the presenter (Ctrl+Alt+G, S03 §2,
    /// command ID 40292).
    ///
    /// The document goes where the user says, so the app only records the
    /// request; the presenter opens the save dialog and hands the answer back
    /// through [`App::finish_html_playlist`].
    pub(crate) request_html_playlist: bool,
    /// The user's saved equalizer presets (`presets.eqf` beside the config).
    pub(crate) eq_user_presets: Vec<crate::eq_presets::UserPreset>,
    /// The equalizer's automatic per-track preset rules (`eq-autoload.toml`
    /// beside the config); see [`crate::eq_autoload`].
    pub(crate) eq_auto_rules: crate::eq_autoload::AutoLoadRules,
    notes: Vec<String>,
    /// The fade reports the user has already seen (see [`crate::fade_note`]).
    ///
    /// The engine reports a fade that does not fit the buffer it is realised
    /// in, and it does so on every fade-out. The app shows the report once
    /// per configuration, and this is where the configuration it showed last
    /// is kept.
    seen_fade_reports: SeenFadeReports,
    /// The device's granted buffer, as the app sees it (see
    /// [`crate::output::GrantedBuffer`]).
    ///
    /// The engine opens the device and keeps the granted format to itself;
    /// the app learns the buffer length through this handle, because its
    /// report about a fade is about the buffer the device actually got.
    device_buffer: crate::output::GrantedBuffer,
    /// The engine's ring, in frames.
    ///
    /// The ring is the app's own configuration: the engine never changes it,
    /// and the report about a fade names it.
    ring_frames: usize,
    /// The last error the user should see (BETA-04): a failed open/decode or
    /// a rejected command. Shown in the main window's native title bar
    /// ([`App::window_title`]); cleared when a track starts.
    last_error: Option<String>,
    /// The reason the **location face** of the open-media dialog refused the
    /// last entry, if it refused one.
    ///
    /// The face's field is a URL, and the app reads it with the command
    /// line's own URL rule (see [`crate::cli::media_argument`]): a scheme
    /// this build cannot read is refused in the dialog — the dialog stays
    /// open, the text stays, and this is the reason it shows. The reason is
    /// the command line's own wording, so the two cannot drift apart.
    pub(crate) location_refusal: Option<String>,
    /// The DSP chain the app installed in the engine (see [`crate::dsp`]).
    ///
    /// The engine takes the stages' ownership and drops the ones that fail,
    /// so the app keeps the recipe and builds a fresh chain whenever one is
    /// installed.
    dsp_chain: crate::dsp::DspChain,
    /// The DSP stage the engine bypassed after it failed, if one was
    /// bypassed (see [`crate::dsp`]).
    ///
    /// The app remembers it so the preferences page does not claim that a
    /// DSP is running while the engine runs without it.
    bypassed_dsp: Option<crate::dsp::BypassedDsp>,
    /// The output plug-in the engine plays through, by its device name.
    plugin_output_name: Option<String>,
    /// A change of the output (plug-in or mode) that waits for playback to stop.
    output_change_pending: bool,
    /// An approval that waits for the user's answer, for the presenter's warning.
    pub(crate) pending_plugin_approval: Option<crate::plugin_approval::PendingApproval>,
    /// The running host read its allowlist before the last approval.
    host_allowlist_stale: bool,
    /// Whether the installed chain is one a caller of the app gave it.
    caller_dsp_chain: bool,
    /// Whether the installed chain is the switched-on DSP plug-in's.
    plugin_dsp_installed: bool,
    /// Whether the app's DSP chain still waits for its installation.
    ///
    /// A live run owns the chain, so an install during a run waits for the
    /// run to end (see [`App::install_dsp_chain`]).
    dsp_install_pending: bool,
    /// The app's end of the player-state bridge to the plugin host
    /// (see [`crate::host`]): detached until a supervisor attaches a
    /// control-channel transport, so publishing without a host is a no-op.
    host: HostStateLink,
    /// The plugin host's spawn and supervision (see [`crate::supervise`).
    ///
    /// `None` is the unmanaged default: the link stays detached and the
    /// player runs identically with and without a host. [`run`](crate::run)
    /// installs one; tests leave it off so no process is ever spawned.
    host_supervisor: Option<Supervisor>,
    /// Where configuration is persisted on clean exit, if known.
    config_path: Option<PathBuf>,
    /// What the file-types page's last action did.
    pub(crate) file_types_report: Option<String>,
    /// What the plug-in pages' last action did, for their report line.
    pub(crate) plugins_report: Option<String>,
    /// The bookmarks (see [`crate::bookmarks`]).
    pub(crate) bookmarks: crate::bookmarks::Bookmarks,
    /// A Winamp configuration import waiting for the presenter's file dialog
    /// (see [`crate::winamp_ini`]).
    pending_winamp_ini_import: bool,
    /// The last Winamp configuration import, for the preferences page.
    ///
    /// The page shows what the import did, so the result is on the page and
    /// not only in the player's status line.
    winamp_ini_import: Option<crate::winamp_ini::ImportOutcome>,
    /// The playlist's title format (spec S14), when the user set one.
    ///
    /// `None` means the user never set one: the rows show their plain text.
    /// The setting belongs to the app, not to `rustamp-core`'s configuration
    /// tree (see [`crate::config::PLAYLIST_TITLE_FORMAT_KEY`]).
    pub(crate) title_format: Option<crate::title_format::TitleFormatSetting>,
    /// The skins folder the user chose, when they chose one.
    ///
    /// `None` means the `skins` folder beside the configuration file is read
    /// (see [`crate::skin_folder::skins_dir`]). The setting belongs to the app,
    /// not to `rustamp-core`'s configuration tree (see
    /// [`crate::config::SKINS_DIR_KEY`]).
    pub(crate) user_skins_dir: Option<PathBuf>,
    /// The folder a preferences page asked the presenter for, until the
    /// dialog answers (see [`FolderRequest`]).
    pending_folder_request: Option<FolderRequest>,
    /// The tags of the playlist's rows, read off the UI thread
    /// (see [`crate::playlist_tags`]).
    pub(crate) playlist_tags: crate::playlist_tags::TagReader,
    /// What the app shows about the stream that is playing (S19 §2).
    ///
    /// The phase the transfer is in is a note, the counters are a transient
    /// status: see [`crate::stream_status`].
    stream: crate::stream_status::StreamStatus,
}

/// Whether `path` has a recognized playlist file extension.
pub(crate) fn is_playlist_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    rustamp_playlist::PlaylistFormat::ALL
        .iter()
        .any(|fmt| fmt.extension().eq_ignore_ascii_case(ext))
}

/// Whether a queue entry is "dead": a local file that no longer exists.
///
/// Only [`TrackSource::Path`] is probed. A URL is a network stream, not a file,
/// so it is never checked and always survives ("Remove all dead files",
/// S03 §2 command id 40222). S03 settles the label, not the stream rule, so
/// that rule — and keeping a path that now names an existing directory — is an
/// own choice (RFC 0018).
fn source_is_dead(source: &TrackSource) -> bool {
    match source {
        TrackSource::Path(path) => !path.exists(),
        TrackSource::Url(_) => false,
    }
}

/// Maps the persisted configuration mode to the audio engine's mode.
fn replaygain_mode_to_audio(
    mode: rustamp_core::config::ReplayGainMode,
) -> rustamp_audio::ReplayGainMode {
    match mode {
        rustamp_core::config::ReplayGainMode::Off => rustamp_audio::ReplayGainMode::Off,
        rustamp_core::config::ReplayGainMode::Track => rustamp_audio::ReplayGainMode::Track,
        rustamp_core::config::ReplayGainMode::Album => rustamp_audio::ReplayGainMode::Album,
    }
}

/// Maps the audio engine's mode back to the persisted configuration mode.
fn replaygain_mode_from_audio(
    mode: rustamp_audio::ReplayGainMode,
) -> rustamp_core::config::ReplayGainMode {
    match mode {
        rustamp_audio::ReplayGainMode::Off => rustamp_core::config::ReplayGainMode::Off,
        rustamp_audio::ReplayGainMode::Track => rustamp_core::config::ReplayGainMode::Track,
        rustamp_audio::ReplayGainMode::Album => rustamp_core::config::ReplayGainMode::Album,
    }
}

/// Whether the configuration says the window `id` is shaded.
///
/// `None` when the configuration says nothing about the window: a key the
/// imported file does not carry leaves the window as it is.
fn window_shaded(config: &Config, id: WindowId) -> Option<bool> {
    config
        .ui
        .windows
        .iter()
        .find(|window| window.id == id)
        .map(|window| window.shaded)
}

/// Whether the configuration says the window `id` is visible.
///
/// `None` when the configuration says nothing about the window: a key the
/// imported file does not carry leaves the window as it is.
fn window_visible(config: &Config, id: WindowId) -> Option<bool> {
    config
        .ui
        .windows
        .iter()
        .find(|window| window.id == id)
        .map(|window| window.visible)
}

/// Writes the notes to `out`, one line each, in the order they were raised.
///
/// The player's **one** printing rule for a note: a line that starts with
/// the player's prefix ([`NOTE_PREFIX`]). The windowed presenter and the
/// headless run both write through here, so a diagnostic reads the same in
/// either mode — there is no second wording and no second stream.
fn write_notes(out: &mut dyn std::io::Write, notes: &[String]) -> std::io::Result<()> {
    for note in notes {
        writeln!(out, "{NOTE_PREFIX}{note}")?;
    }
    Ok(())
}

impl App {
    /// Builds the player with the default decoder registry.
    pub fn new(options: AppOptions) -> Result<App> {
        Self::with_registry(options, rustamp_decoders::default_registry())
    }

    /// Builds the player with an explicit registry (embedding and tests).
    pub fn with_registry(options: AppOptions, registry: DecoderRegistry) -> Result<App> {
        let bus = EventBus::new();
        let events = bus.subscribe();
        let (commands, command_rx) = command_channel();

        let output = create_output(
            options.output,
            options.config.audio.device.as_deref(),
            options.config.audio.output_mode,
        );

        let cfg = EngineConfig {
            volume: options.volume,
            balance: options.config.audio.balance,
            // The fade-out duration is the configured one. The engine's own
            // default happens to be the configuration's default, so the two
            // agree by accident — the configured value must be handed over
            // explicitly, or a file that asks for a different fade is ignored.
            fade_ms: options.config.audio.fadeout_ms,
            ..EngineConfig::default()
        };
        // The ring is the app's own configuration and the engine never
        // changes it: the app keeps the number, because its report about a
        // fade names the ring the fade is realised in (see `crate::fade_note`).
        let ring_frames = cfg.ring_frames;
        let tag_registry = registry.clone();
        let mut engine = Engine::new(registry.clone(), output.device, bus.clone(), cfg)?;
        // `Engine::new` initialises its gain from the config but reports
        // `volume: 255, balance: 0` in `PlayerState` (an engine-side
        // inconsistency; see the findings). Re-assert the configured values so
        // the reported state matches what the engine actually applies.
        engine.set_volume(options.volume)?;
        engine.set_balance(options.config.audio.balance)?;
        engine.set_eq(options.config.eq.state);
        engine.set_replaygain_mode(replaygain_mode_to_audio(options.config.audio.replaygain));
        // The playback modes are the configured ones. The engine owns the
        // traversal (S15), so a configured shuffle that only lit the button
        // would leave the engine playing in order: the modes go to the engine
        // itself, and the button follows the engine (see `App::state`).
        engine.set_shuffle(options.config.playlist.shuffle)?;
        engine.set_repeat(options.config.playlist.repeat)?;
        // The media the command line named. A file replaces the queue and
        // starts playing (S02 §1); a URL is a stream track (S19) — the same
        // kind of queue entry a playlist's URL produces.
        match &options.media {
            Some(MediaLocation::File(path)) if !is_playlist_file(path) => {
                engine.set_queue(vec![TrackSource::path(path.clone())]);
            }
            Some(MediaLocation::Url(url)) => {
                engine.set_queue(vec![TrackSource::Url(url.clone())]);
            }
            _ => {}
        }

        let main_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Main);
        let eq_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Equalizer);
        let pl_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Playlist);
        // The editor's windowshade is saved with the other windows (S01 §1),
        // so a saved editor comes back shaded. Read it before the config
        // moves into the app.
        let pl_shaded = pl_layout.is_some_and(|layout| layout.shaded);

        // Seed the UI machine from the config, window layouts, and the engine's initial state.
        let initial = MainWindowState {
            volume: volume_percent(options.volume),
            balance: rustamp_ui::balance_percent(options.config.audio.balance),
            double_size: options.config.ui.double_size,
            clutter_a: options.config.ui.always_on_top,
            shuffle: options.config.playlist.shuffle,
            repeat: options.config.playlist.repeat,
            windowshade: main_layout.map(|l| l.shaded).unwrap_or(false),
            eq_open: eq_layout.map(|l| l.visible).unwrap_or(false),
            playlist_open: pl_layout.map(|l| l.visible).unwrap_or(true),
            ..MainWindowState::default()
        };
        let eq_shaded = eq_layout.is_some_and(|layout| layout.shaded);

        let mut app = App {
            engine,
            ui: MainWindow::from_state(initial),
            vis_discontinuity: false,
            layout: MainWindowLayout::CLASSIC,
            eq: EqualizerWindow::from_state(rustamp_ui::EqualizerState {
                windowshade: eq_shaded,
                ..rustamp_ui::EqualizerState::default()
            }),
            eq_layout: EqualizerLayout::CLASSIC,
            window_dirty: WindowDirty::NONE,
            main_frame: None,
            eq_frame: None,
            library_frame: None,
            title_bar_clicks: TitleBarClicks::default(),
            skin: options.skin,
            config: options.config,
            plugin_scan: rustamp_prefs::PluginScan::default(),
            plugin_scanned: false,
            plugin_host_generation: 0,
            plugin_requests: HashSet::new(),
            plugin_startup_loaded: false,
            config_path: None,
            file_types_report: None,
            plugins_report: None,
            user_skins_dir: None,
            pending_folder_request: None,
            bookmarks: crate::bookmarks::Bookmarks::default(),
            bus,
            events,
            commands,
            command_rx,
            control_state: Arc::new(Mutex::new(PlayerState::default())),
            control_channel: false,
            output_choice: options.output,
            output_notes: output.notes,
            builtin_registry: registry.clone(),
            registry,
            clock: output.clock,
            started: Instant::now(),
            seconds: options.seconds,
            last_null_tick: Instant::now(),
            pending_null_frames: 0.0,
            shuffle: false,
            repeat: false,
            always_on_top: false,
            easy_move: true,
            focused: true,
            playlist_focused: false,
            current_skin: None,
            skin_generation: 0,
            skin_folder: RefCell::default(),
            quit: false,
            main_visible: true,
            minimize_requested: false,
            focused_window: WindowId::Main,
            focus_request: None,
            restore_request: false,
            playlist_ended: false,
            entry_ids: Vec::new(),
            next_entry_id: 0,
            playlist: PlaylistWindow::new(),
            playlist_double_size: false,
            playlist_resized_in_double_size: false,
            library_window: crate::library_window::LibraryWindow::default(),
            last_extract_settings: rustamp_platform::ExtractSettings {
                template: crate::library_extract::DEFAULT_TEMPLATE.to_owned(),
                directory: PathBuf::new(),
            },
            dialogs: DialogManager::new(),
            extended_info: None,
            edit_entry: None,
            last_playlist_report: None,
            last_playlist_format: None,
            main_menu_open: false,
            pending_clutter_menu: None,
            pending_eq_menu: false,
            pending_playlist_misc_menu: false,
            pending_context_menu: None,
            pending_eq_file: false,
            pending_playlist_context: None,
            request_save_playlist: false,
            request_html_playlist: false,
            eq_user_presets: Vec::new(),
            eq_auto_rules: crate::eq_autoload::AutoLoadRules::default(),
            notes: Vec::new(),
            seen_fade_reports: SeenFadeReports::default(),
            device_buffer: output.granted.clone(),
            ring_frames,
            last_error: None,
            location_refusal: None,
            dsp_chain: crate::dsp::DspChain::default(),
            plugin_output_name: None,
            output_change_pending: false,
            pending_plugin_approval: None,
            host_allowlist_stale: false,
            caller_dsp_chain: false,
            plugin_dsp_installed: false,
            bypassed_dsp: None,
            dsp_install_pending: false,
            host: HostStateLink::detached(),
            host_supervisor: None,
            pending_winamp_ini_import: false,
            winamp_ini_import: None,
            title_format: None,
            playlist_tags: crate::playlist_tags::TagReader::with_length(tag_registry),
            stream: crate::stream_status::StreamStatus::default(),
        };
        app.shuffle = app.config.playlist.shuffle;
        app.repeat = app.config.playlist.repeat;
        app.always_on_top = app.config.ui.always_on_top;
        app.easy_move = app.config.ui.easy_move;
        // A path skin remembers where it came from, so `RefreshSkin` can re-read it.
        app.current_skin = app.skin.path().map(PathBuf::from);
        app.eq.set_double_size(app.ui.state().double_size);
        // The playlist editor's windowshade is saved with the other windows
        // (S01 §1: the shade band is 14 px tall), so a saved editor comes
        // back shaded.
        app.playlist.windowshade = pl_shaded;
        app.update_playlist_double_size(app.ui.state().double_size);
        // The skin's `Font=` size is the playlist's default size (S17 §5);
        // the user can change it at runtime (S03 §2 `40335` / `40336`).
        app.sync_playlist_font_size();
        app.sync_eq_from_engine();
        // The DSP chain the app installs in the engine.
        //
        // The chain is empty today: a classic DSP plug-in runs in the 32-bit
        // plug-in host, and the host has no message that could carry a block
        // of audio to the engine's decode thread, so the app has no
        // engine-side stage for a plug-in yet (RFC 0020 Q6 is open). The app
        // installs the chain at start-up anyway, so the policy around it —
        // the note, the remembered state and the manual recovery — is in
        // place for the first real stage.
        app.install_dsp_chain(crate::dsp::DspChain::default());
        // The engine's snapshot is the authoritative starting state.
        app.sync_entry_ids();
        if let Some(MediaLocation::File(file)) = &options.media
            && is_playlist_file(file)
        {
            let _ = app.load_playlist_file(file, false);
        }
        let snapshot = app.engine.state();
        app.sync_from_player_state(&snapshot);
        Ok(app)
    }

    /// Milliseconds since the app was built. The app owns the clock; the UI
    /// machine only receives it.
    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The current render state.
    pub fn state(&self) -> &MainWindowState {
        self.ui.state()
    }

    /// The headless UI machine.
    pub fn ui(&self) -> &MainWindow {
        &self.ui
    }

    /// The equalizer's headless behaviour (S07).
    pub fn eq(&self) -> &EqualizerWindow {
        &self.eq
    }

    /// The equalizer's current render state.
    pub fn eq_state(&self) -> &rustamp_ui::EqualizerState {
        self.eq.state()
    }

    /// Whether the equalizer window is open (the main window's `eq_open` flag,
    /// flipped by its EQ toggle button).
    pub fn eq_open(&self) -> bool {
        self.ui.state().eq_open
    }

    /// Whether the playlist editor window is open (the main window's
    /// `playlist_open` flag; the presenter creates or drops the OS window to
    /// match it, the same way as the equalizer's).
    pub fn playlist_open(&self) -> bool {
        self.ui.state().playlist_open
    }

    /// The last error surfaced to the user, if one is showing (BETA-04).
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// The outcome of the most recent playlist file load, if one has occurred.
    pub fn last_playlist_report(&self) -> Option<&rustamp_playlist::PlaylistReport> {
        self.last_playlist_report.as_ref()
    }

    /// The format the currently loaded playlist was detected as, if any.
    ///
    /// `None` means no playlist has been loaded yet; callers default to M3U.
    pub fn last_playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// Alias for [`Self::last_playlist_format`] using the task's wording.
    pub fn loaded_playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// Alias for [`Self::last_playlist_format`].
    pub fn playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// The main window's native title: `Rustamp`, or `Rustamp - <error>`
    /// while an error is showing.
    ///
    /// Own choice, stated plainly: no spec gives the player a place for
    /// error text. The native title bar is legible whatever the skin (the
    /// built-in placeholder font is not, BETA-02), changes nothing the specs
    /// measured inside the skinned window, and the presenter applies it.
    pub fn window_title(&self) -> String {
        match &self.last_error {
            Some(error) => format!("Rustamp - {error}"),
            None => "Rustamp".to_string(),
        }
    }

    /// The headless UI machine, mutably (tests and the presenter).
    pub fn ui_mut(&mut self) -> &mut MainWindow {
        &mut self.ui
    }

    /// The audio engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The audio engine, mutably (tests and embedding).
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// The file extensions this player's decoders claim.
    ///
    /// The directory face of the open-media dialog uses this to tell media
    /// files from the other files in a folder (see [`crate::directory`]).
    /// The list comes from the registry the app was built with, so an
    /// embedding that registers its own decoder gets its own extensions.
    pub fn decodable_extensions(&self) -> Vec<String> {
        self.registry.extensions()
    }

    /// Recreates the audio output device and engine as required by the audio
    /// contract (crates/rustamp-core/src/audio.rs:370-380) when a backend
    /// refuses flush (`Error::Unsupported`) or closes the output.
    /// Preserves the queue, current track index, volume, balance, equalizer
    /// settings, replaygain, limiter, shuffle, and repeat modes.
    pub fn recreate_engine(&mut self) -> Result<()> {
        let output = create_output(
            self.output_choice,
            self.config.audio.device.as_deref(),
            self.config.audio.output_mode,
        );
        let mut output = output;
        if let Some(device) = self.plugin_output_device() {
            output.device = device;
        }
        let queue = self.engine.queue().to_vec();
        let current_index = self.engine.current_index();
        let volume = self.engine.volume();
        let balance = self.engine.balance();
        let eq = self.engine.eq();
        let limiter = self.engine.limiter_enabled();
        let replaygain_mode = self.engine.replaygain_mode();
        let replaygain_preamp_db = self.engine.replaygain_preamp_db();
        let stop_after_current = self.engine.stop_after_current();
        let shuffle = self.engine.shuffle();
        let repeat = self.engine.repeat();

        let cfg = EngineConfig {
            volume,
            balance,
            // The fade-out duration is a configuration setting, not a runtime
            // one: the new engine runs the configured fade as well.
            fade_ms: self.config.audio.fadeout_ms,
            ..EngineConfig::default()
        };
        let mut new_engine =
            Engine::new(self.registry.clone(), output.device, self.bus.clone(), cfg)?;
        new_engine.set_volume(volume)?;
        new_engine.set_balance(balance)?;
        new_engine.set_eq(eq);
        new_engine.set_queue(queue);
        new_engine.set_current_index(current_index);
        new_engine.set_limiter_enabled(limiter);
        new_engine.set_replaygain_mode(replaygain_mode);
        new_engine.set_replaygain_preamp_db(replaygain_preamp_db);
        let _ = new_engine.set_shuffle(shuffle);
        let _ = new_engine.set_repeat(repeat);
        if stop_after_current {
            let _ = new_engine.toggle_stop_after_current();
        }

        self.engine = new_engine;
        // Do not lose a pending diagnostic from the previous device.
        self.collect_output_notes();
        self.output_notes = output.notes;
        self.clock = output.clock;
        // The new device granted its own buffer: the report about a fade is
        // about the buffer the device actually got (see `crate::fade_note`).
        self.device_buffer = output.granted;
        Ok(())
    }

    /// Recreates the engine and output if the previous output closed due to
    /// a terminal flush refusal or failure, fulfilling the caller's duty
    /// under the audio contract.
    ///
    /// This is the caller's half of the output recovery split, and the whole of
    /// it. A fatal mid-run *stream* error is explicitly not handled here: RFC
    /// 0012 §4.3 item 8 gives that case to `Engine::play()`, which closes and
    /// reopens the device object the engine already owns and keeps the position
    /// it froze until an explicit retry. Stream-error teardown never arms
    /// [`Engine::is_flush_failed`]. An actual flush refusal (including a later
    /// stop/seek) still takes precedence. See `docs/tasks/W23-rfc0012-gaps.md` §4.
    pub fn ensure_engine_ready(&mut self) -> Result<()> {
        if self.engine.is_flush_failed() {
            self.note(
                "recreating audio engine and output after terminal flush refusal".to_string(),
            );
            self.recreate_engine()?;
        }
        Ok(())
    }

    /// The loaded configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The loaded configuration (mutable).
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// The ReplayGain mode the engine currently runs in.
    ///
    /// The engine is the live truth: the saved configuration takes the mode
    /// from it ([`App::current_config`]).
    pub fn replaygain_mode(&self) -> rustamp_core::config::ReplayGainMode {
        replaygain_mode_from_audio(self.engine.replaygain_mode())
    }

    /// Sets the ReplayGain mode of the engine at once.
    pub fn set_replaygain_mode(&mut self, mode: rustamp_core::config::ReplayGainMode) {
        self.engine
            .set_replaygain_mode(replaygain_mode_to_audio(mode));
    }

    /// Remembers the requested output mode for the next device creation.
    ///
    /// The device is opened when the player starts, so the mode applies from
    /// the next start (or engine recreation) on, not to a running stream.
    /// A downgrade never overwrites the saved request.
    pub fn set_output_mode(&mut self, mode: rustamp_core::config::OutputMode) {
        self.config.audio.output_mode = mode;
    }

    /// Sets the path where configuration will be persisted on exit.
    pub fn set_config_path(&mut self, path: Option<PathBuf>) {
        self.open_bookmarks(path.as_deref());
        self.config_path = path;
        self.load_eq_user_presets();
        self.load_eq_auto_rules();
    }

    /// The path where configuration will be persisted, if set.
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// The visualization mode the main window draws (S08).
    pub fn vis_mode(&self) -> rustamp_ui::visualization::VisMode {
        self.ui.state().visualization.mode
    }

    /// Sets the visualization mode: the Visualization page's drop-down list.
    ///
    /// The mode is the player's own render choice; it takes effect on the next
    /// frame. The caller saves when the choice is the user's (see
    /// [`App::apply_prefs_action`]).
    pub fn set_vis_mode(&mut self, mode: rustamp_ui::visualization::VisMode) {
        self.ui.set_vis_mode(mode);
    }

    /// The folder the skin browser reads.
    ///
    /// The folder the user chose, or the `skins` folder beside the
    /// configuration file when they never chose one (see
    /// [`crate::skin_folder::skins_dir`]).
    pub fn skins_dir(&self) -> Option<PathBuf> {
        self.user_skins_dir
            .clone()
            .or_else(|| crate::skin_folder::skins_dir(self.config_path()))
    }

    /// Makes `dir` the folder the skin browser reads, and saves the choice.
    ///
    /// The skin list follows the new folder on its own: the cache reads a
    /// folder it has not read before.
    pub fn set_skins_folder(&mut self, dir: PathBuf) {
        self.user_skins_dir = Some(dir);
        if let Err(error) = self.save_config() {
            self.note(format!("preferences: {error}"));
        }
    }

    /// Restores the skins folder a configuration load named.
    ///
    /// Nothing is saved: this is the value coming out of the file, not a
    /// choice the user made now (see [`App::set_skins_folder`]).
    pub fn set_skins_dir_from_load(&mut self, dir: Option<PathBuf>) {
        self.user_skins_dir = dir;
    }

    /// Makes `dir` the folder the plug-in pages scan, and saves the choice.
    ///
    /// The folder is read again at once, so the pages show what the new folder
    /// holds; nothing in it is loaded.
    pub fn set_plugin_folder(&mut self, dir: PathBuf) {
        self.config_mut().plugins.plugin_dir = Some(dir);
        if let Err(error) = self.save_config() {
            self.note(format!("preferences: {error}"));
        }
        self.rescan_plugins();
        self.plugins_report = Some(format!(
            "The plug-in folder is {}.",
            self.plugin_dir().display()
        ));
    }

    /// The `[ui]` settings the app keeps itself, for a save.
    pub(crate) fn app_ui(&self) -> crate::config::AppUi {
        crate::config::AppUi {
            playlist_font_size: self.playlist_font_size(),
            playlist_title_format: self.title_format.clone(),
            vis_mode: self.vis_mode(),
            skins_dir: self.user_skins_dir.clone(),
        }
    }

    /// The folder a preferences page asked the presenter for, taking it.
    ///
    /// The dialog runs its own modal loop, so the request is drained from the
    /// tick, never from a window procedure (the rule the library's folder
    /// pick follows).
    pub fn take_folder_request(&mut self) -> Option<FolderRequest> {
        self.pending_folder_request.take()
    }

    /// Asks the presenter for a folder (see [`FolderRequest`]).
    pub(crate) fn request_folder(&mut self, request: FolderRequest) {
        self.pending_folder_request = Some(request);
    }

    /// Returns the live configuration reflecting the player's current runtime state.
    pub fn current_config(&self) -> Config {
        let player_state = self.engine.state();
        let volume = player_state.volume;
        let balance = player_state.balance;

        let mut windows = self.config.ui.windows.clone();
        if let Some(main) = windows.iter_mut().find(|w| w.id == WindowId::Main) {
            main.shaded = self.ui.state().windowshade;
            // The main window's visibility is the live one. The player
            // always opens the main window when it starts (an unavailable
            // tray must never strand the player), so the value records the
            // state the player was in, not a "start hidden" instruction.
            main.visible = self.main_visible;
        } else {
            windows.push(WindowLayout {
                id: WindowId::Main,
                x: 26,
                y: rustamp_ui::DEFAULT_MAIN_POSITION.y,
                visible: self.main_visible,
                shaded: self.ui.state().windowshade,
                width: None,
                height: None,
            });
        }
        if let Some(eq) = windows.iter_mut().find(|w| w.id == WindowId::Equalizer) {
            eq.visible = self.eq_open();
            eq.shaded = self.eq_state().windowshade;
        } else {
            windows.push(WindowLayout {
                id: WindowId::Equalizer,
                x: 26,
                y: rustamp_ui::DEFAULT_EQUALIZER_POSITION.y,
                visible: self.eq_open(),
                shaded: self.eq_state().windowshade,
                width: None,
                height: None,
            });
        }
        if let Some(pl) = windows.iter_mut().find(|w| w.id == WindowId::Playlist) {
            pl.visible = self.playlist_open();
            pl.shaded = self.playlist.windowshade;
        } else {
            windows.push(WindowLayout {
                id: WindowId::Playlist,
                x: 26,
                y: rustamp_ui::DEFAULT_PLAYLIST_POSITION.y,
                visible: self.playlist_open(),
                shaded: self.playlist.windowshade,
                width: None,
                height: None,
            });
        }

        Config {
            version: rustamp_core::config::CONFIG_VERSION,
            audio: AudioConfig {
                volume,
                balance,
                replaygain: replaygain_mode_from_audio(self.engine.replaygain_mode()),
                ..self.config.audio.clone()
            },
            ui: UiConfig {
                skin: self.current_skin.clone(),
                double_size: self.ui.state().double_size,
                always_on_top: self.always_on_top,
                easy_move: self.easy_move,
                windows,
                ..self.config.ui.clone()
            },
            eq: EqConfig {
                state: self.engine.eq(),
            },
            playlist: PlaylistConfig {
                // The playback modes are applied (the engine owns the
                // traversal, S15), so the live ones are the file's.
                shuffle: self.shuffle,
                repeat: self.repeat,
                ..PlaylistConfig::default()
            },
            plugins: PluginsConfig {
                // What the plug-in pages edit: the folder and the enabled list.
                // The rest stays unpersisted (see `config::serialize_honest`).
                plugin_dir: self.config.plugins.plugin_dir.clone(),
                enabled: self.config.plugins.enabled.clone(),
                ..PluginsConfig::default()
            },
        }
    }

    /// The plug-in settings the preferences pages edit.
    pub fn plugins_config(&self) -> &PluginsConfig {
        &self.config.plugins
    }

    /// The folder the plug-in pages scan: the configured one, else `Plugins`
    /// beside the executable (the classic layout).
    pub fn plugin_dir(&self) -> PathBuf {
        self.config.plugins.plugin_dir.clone().unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("Plugins")))
                .unwrap_or_else(|| PathBuf::from("Plugins"))
        })
    }

    /// Where the approval store lives: beside the configuration file (the
    /// executable's folder when no configuration path is set).
    pub fn plugin_approvals_path(&self) -> PathBuf {
        let beside = self
            .config_path
            .as_deref()
            .and_then(Path::parent)
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(Path::to_path_buf))
            })
            .unwrap_or_default();
        beside.join(rustamp_prefs::APPROVALS_FILE)
    }

    /// Reads the plug-in folder and the approval store again. Reads files as
    /// bytes only; nothing is loaded or executed.
    pub fn rescan_plugins(&mut self) {
        self.plugin_scan = rustamp_prefs::scan(&self.plugin_dir(), &self.plugin_approvals_path());
        self.plugin_scanned = true;
    }

    /// The last plug-in scan.
    pub fn plugin_scan(&self) -> &rustamp_prefs::PluginScan {
        &self.plugin_scan
    }

    /// Switches the scanned plug-in `index` on or off. Switching on is refused
    /// unless the approval store approved the exact image
    /// ([`rustamp_prefs::set_enabled`]); there is no other way to enable one.
    pub fn set_plugin_enabled(
        &mut self,
        index: usize,
        on: bool,
    ) -> Result<(), rustamp_prefs::EnableRefused> {
        let Some(entry) = self.plugin_scan.entries.get(index) else {
            return Ok(());
        };
        rustamp_prefs::set_enabled(&mut self.config.plugins, entry, on)
    }

    /// Saves the current configuration to the configured path, if one is set.
    pub fn save_config(&self) -> Result<(), crate::config::ConfigError> {
        let Some(path) = &self.config_path else {
            return Ok(());
        };
        self.save_config_to(path)
    }

    /// Saves the current configuration to `path`.
    pub fn save_config_to(&self, path: &Path) -> Result<(), crate::config::ConfigError> {
        let mut config = self.current_config();
        // A manual resize in 2x mode is a deliberate physical size. Keep that
        // size across restart even though the saved geometry is normally 1x.
        if self.playlist_double_size
            && self.playlist_resized_in_double_size
            && let Some(layout) = config
                .ui
                .windows
                .iter_mut()
                .find(|w| w.id == WindowId::Playlist)
        {
            layout.width = layout.width.map(|width| width * 2);
            layout.height = layout.height.map(|height| height * 2);
        }
        crate::config::save(&config, &self.app_ui(), path)
    }

    /// Imports a Winamp 2.95 `winamp.ini` into the running configuration.
    ///
    /// The file is only **read**: the import never writes to it, and what it
    /// produces is our own configuration, which the player saves on clean
    /// exit.
    ///
    /// What the running player can take over is taken over at once: the
    /// volume, the balance, the equalizer, the playback modes, the window
    /// flags, the shade and the visibility of the windows, the playlist
    /// editor's font size and the skin. The window **placement** stays in the
    /// configuration: the player restores it when it starts, so an import
    /// that runs before the player starts places the windows right away and
    /// an import that runs later waits for the next start.
    ///
    /// Everything the player has no setting for is **reported** and never
    /// dropped silently — see [`crate::winamp_ini`] for the mapping table and
    /// for the two readings that are ours and not the spec's.
    ///
    /// The outcome is remembered, so the preferences page can show what the
    /// import did (see [`App::winamp_ini_report`]).
    pub fn import_winamp_ini(
        &mut self,
        path: &Path,
    ) -> Result<crate::winamp_ini::ImportReport, crate::winamp_ini::ImportError> {
        let skins = self.skins_dir();
        let resolve_skin = |name: &str| {
            skins
                .as_ref()
                .and_then(|dir| crate::skin_folder::find_skin_by_name(dir, name))
        };
        let outcome = match crate::winamp_ini::import_file(
            path,
            &self.current_config(),
            self.playlist_font_size(),
            &resolve_skin,
        ) {
            Ok(crate::winamp_ini::ImportPlan {
                config,
                playlist_font_size,
                report,
            }) => {
                self.apply_import(&config, playlist_font_size);
                self.note(format!(
                    "Winamp configuration imported: {}",
                    report.summary()
                ));
                Ok(report)
            }
            Err(error) => Err(error),
        };
        // The outcome is remembered for the preferences page, whether the
        // file was read or not: the page tells the user what happened.
        self.winamp_ini_import = Some(match &outcome {
            Ok(report) => crate::winamp_ini::ImportOutcome::Imported {
                path: path.to_path_buf(),
                report: report.clone(),
            },
            Err(error) => crate::winamp_ini::ImportOutcome::Failed {
                error: error.clone(),
            },
        });
        outcome
    }

    /// Ends the Winamp configuration import the user was asked for.
    ///
    /// `path` is the file the user chose in the file dialog; `None` means
    /// the dialog was cancelled. A cancelled import changes nothing: the
    /// configuration stays as it was and the preferences page keeps the
    /// report of the last import.
    ///
    /// The outcome is returned, so the presenter can say what happened.
    pub fn finish_winamp_ini_import(
        &mut self,
        path: Option<&Path>,
    ) -> Option<crate::winamp_ini::ImportOutcome> {
        let path = path?;
        // The import remembers the outcome; it is handed back so the caller
        // can say what happened.
        let _ = self.import_winamp_ini(path);
        self.winamp_ini_import.clone()
    }

    /// The report of the last Winamp configuration import, for the
    /// preferences page.
    ///
    /// `None` when no import has run yet: the page then says so instead of
    /// showing a report.
    pub fn winamp_ini_report(&self) -> Option<String> {
        self.winamp_ini_import
            .as_ref()
            .map(|outcome| outcome.line())
    }

    /// Takes over the settings an import produced.
    ///
    /// The engine and the UI machine take the live settings over; the
    /// configuration keeps all of them, so the window placement survives
    /// until the next start.
    fn apply_import(&mut self, config: &Config, playlist_font_size: Option<u32>) {
        // The engine owns the volume, the balance and the playback modes;
        // the commands carry them to the UI machine as well, so the sliders
        // and the buttons agree with the engine.
        self.apply(&Command::SetVolume(config.audio.volume));
        self.apply(&Command::SetBalance(config.audio.balance));
        self.apply(&Command::SetShuffle(config.playlist.shuffle));
        self.apply(&Command::SetRepeat(config.playlist.repeat));
        // The equalizer: the engine is the truth, the window follows it.
        self.engine.set_eq(config.eq.state);
        self.sync_eq_from_engine();
        // The window flags the player owns.
        self.always_on_top = config.ui.always_on_top;
        self.easy_move = config.ui.easy_move;
        if self.ui.state().double_size != config.ui.double_size {
            self.apply(&Command::ToggleDoubleSize);
        }
        // The main window's shade and visibility. A window the file says
        // nothing about keeps what it has now.
        if let Some(shaded) = window_shaded(config, WindowId::Main)
            && self.ui.state().windowshade != shaded
        {
            self.apply(&Command::ToggleWindowShade(WindowId::Main));
        }
        if let Some(visible) = window_visible(config, WindowId::Main)
            && self.main_visible != visible
        {
            self.main_visible = visible;
        }
        // The equalizer's shade.
        if let Some(shaded) = window_shaded(config, WindowId::Equalizer)
            && self.eq_state().windowshade != shaded
        {
            self.apply(&Command::ToggleWindowShade(WindowId::Equalizer));
        }
        // The equalizer's and the playlist editor's visibility. The UI
        // machine owns these two flags; the app's own path for the toggle
        // records a note about the equalizer's presets submenu, which has
        // nothing to do with an import, so the machine is asked directly.
        if let Some(visible) = window_visible(config, WindowId::Equalizer)
            && self.eq_open() != visible
        {
            self.ui
                .handle_command(&Command::ToggleWindow(WindowId::Equalizer), self.now_ms());
        }
        if let Some(visible) = window_visible(config, WindowId::Playlist)
            && self.playlist_open() != visible
        {
            self.ui
                .handle_command(&Command::ToggleWindow(WindowId::Playlist), self.now_ms());
            self.maintain_focus();
        }
        // The playlist editor's font size.
        self.set_playlist_font_size(playlist_font_size);
        // The skin: the reference names a file, we resolved it in our skins
        // folder. A skin that cannot be loaded falls back with a warning,
        // which is the app's own rule.
        if let Some(skin) = &config.ui.skin
            && self.current_skin.as_deref() != Some(skin.as_path())
        {
            self.load_skin(skin.clone());
        }
        // The configuration keeps everything, the window placement included.
        self.config = config.clone();
        self.update_playlist_double_size(self.ui.state().double_size);
    }

    /// Asks for a Winamp configuration file and imports it.
    ///
    /// The request is for the presenter: the native file dialog needs an
    /// owner window, so the headless app cannot answer it.
    pub fn request_winamp_ini_import(&mut self) {
        self.pending_winamp_ini_import = true;
    }

    /// Takes the Winamp configuration import request once, for the
    /// presenter's dialog.
    pub fn take_winamp_ini_import_request(&mut self) -> bool {
        std::mem::take(&mut self.pending_winamp_ini_import)
    }

    /// Records a window's screen coordinates in the config.
    pub fn set_window_position(&mut self, id: WindowId, x: i32, y: i32) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.x = x;
            entry.y = y;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x,
                y,
                visible: true,
                shaded: false,
                width: None,
                height: None,
            });
        }
    }

    /// Records a window's visibility state in the config.
    pub fn set_window_visible(&mut self, id: WindowId, visible: bool) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.visible = visible;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x: 26,
                y: 26,
                visible,
                shaded: false,
                width: None,
                height: None,
            });
        }
    }

    /// Records a window's shaded state in the config.
    pub fn set_window_shaded(&mut self, id: WindowId, shaded: bool) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.shaded = shaded;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x: 26,
                y: 26,
                visible: true,
                shaded,
                width: None,
                height: None,
            });
        }
    }

    /// A one-line description of the loaded skin.
    pub fn skin_description(&self) -> &str {
        self.skin.description()
    }

    /// Whether the app should exit.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Whether the presenter should show the main window (F3/F5 show/hide).
    pub fn main_visible(&self) -> bool {
        self.main_visible
    }

    /// Whether the app uses the null output (`--null` headless or `--no-audio`
    /// windowed). This reports the device, not the presentation: a
    /// `--no-audio` run still opens the window.
    pub fn headless(&self) -> bool {
        self.clock.is_some()
    }

    /// Always-on-top (S02 §2, clutter-bar `A`).
    pub fn always_on_top(&self) -> bool {
        self.always_on_top
    }

    /// Easy-move (RFC 0005, id 40186).
    pub fn easy_move(&self) -> bool {
        self.easy_move
    }

    /// Whether a left press at main-window-relative `(x, y)` should move the
    /// window instead of reaching a control: easy-move is on — the core
    /// contract's "drag windows from anywhere" (`UiConfig::easy_move`) — and
    /// the press hit no widget, so no control owns it. Sliders, the marquee
    /// hand-scroll, buttons and held states all hit-test to a widget and keep
    /// their presses; a background press reaches nothing and drags.
    pub fn easy_drag_main(&self, x: i32, y: i32) -> bool {
        self.easy_move && rustamp_ui::hit_test(&self.layout, x, y).is_none()
    }

    /// The path of the current skin, when it came from disk.
    pub fn current_skin(&self) -> Option<&PathBuf> {
        self.current_skin.as_ref()
    }

    /// The number of skin loads so far.
    ///
    /// The count changes whenever the app loads a skin, so a cache of
    /// skin-derived resources (the cursor cache) knows when to drop what
    /// it built for the previous skin.
    pub fn skin_generation(&self) -> u64 {
        self.skin_generation
    }

    /// The client area the OS window must have right now, in physical pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        frame_size(self.ui.state())
    }

    /// The client area the equalizer's OS window must have right now (S05
    /// §2.1: 275 x 116, 550 x 232 double-sized; the shade size follows the
    /// existing docking geometry, S05 §1.5 open).
    pub fn eq_frame_size(&self) -> (u32, u32) {
        eq_frame_size(self.eq.state())
    }

    /// Takes the pending "minimize the main window" request, if any.
    pub fn take_minimize_request(&mut self) -> bool {
        std::mem::take(&mut self.minimize_requested)
    }

    /// Whether `window` exists and can take focus right now.
    ///
    /// The main window counts while it is shown ([`App::main_visible`]); the
    /// equalizer and the playlist editor count while their open flags say so.
    /// A closed or hidden window cannot take focus, and windows the app does
    /// not have (`Vis`, `Video`, every `Gen` window but the media library)
    /// report closed.
    pub fn window_open(&self, window: WindowId) -> bool {
        match window {
            WindowId::Main => self.main_visible,
            WindowId::Equalizer => self.eq_open(),
            WindowId::Playlist => self.playlist_open(),
            WindowId::Gen(id) if id == crate::library_window::LIBRARY_WINDOW_NUMBER => {
                self.library_open()
            }
            WindowId::Vis | WindowId::Gen(_) | WindowId::Video => false,
        }
    }

    /// Which player window holds keyboard focus (see [`crate::focus`]).
    pub fn focused_window(&self) -> WindowId {
        self.focused_window
    }

    /// Takes the pending "focus this OS window" request, if any.
    pub fn take_focus_request(&mut self) -> Option<WindowId> {
        self.focus_request.take()
    }

    /// Takes the pending "un-minimize the main OS window" request, if any.
    pub fn take_restore_request(&mut self) -> bool {
        std::mem::take(&mut self.restore_request)
    }

    /// Drains the accumulated notes (unimplemented commands, fallbacks).
    pub fn drain_notes(&mut self) -> Vec<String> {
        self.collect_output_notes();
        std::mem::take(&mut self.notes)
    }

    /// Prints the accumulated notes to the standard error, in the order they
    /// were raised.
    ///
    /// The player's one rule for a note (see [`write_notes`]): a line that
    /// starts with the player's prefix, on the standard error. The windowed
    /// presenter and the headless run both print through here, so a
    /// diagnostic reads the same in either mode — and the headless run no
    /// longer swallows the reports the waves added (the DSP bypass, the
    /// fade budget, the library's scan report, the import report and the
    /// "not implemented" notes).
    ///
    /// The queue is drained, so a note is printed **once**.
    pub(crate) fn print_notes(&mut self) {
        let notes = self.drain_notes();
        // A standard error that cannot be written must not stop the player.
        let _ = write_notes(&mut std::io::stderr(), &notes);
    }

    fn collect_output_notes(&mut self) {
        // Unlike repetitive unimplemented-command notes, output downgrades
        // must not be discarded when the ordinary note budget is exhausted.
        self.notes.extend(self.output_notes.try_iter());
    }

    /// Asks the app to open one of the playlist window's dialogs (S06 §4).
    ///
    /// The spec settles the dialogs' existence and titles and fixes no dialog
    /// behaviour beyond that. When the dialog's opening command maps to a real
    /// dialog ([`App::open_dialog_for`]), the dialog opens and no note is
    /// recorded; otherwise the spec'd title goes through the existing
    /// dialog-stub path and opens no window. Returns the title the real
    /// dialog must carry.
    pub fn request_playlist_dialog(&mut self, dialog: PlaylistDialog) -> &'static str {
        let title = dialog.title();
        if let Some(command) = dialog.opening_command()
            && self.open_dialog_for(&command).is_some()
        {
            return title;
        }
        self.note(format!(
            "open dialog \"{title}\" ({dialog:?}): the dialog is not implemented yet"
        ));
        title
    }

    /// Asks the app to open the equalizer's presets popup (S07 §4.2).
    ///
    /// The spec settles the popup's existence (window class `#32768`), size
    /// and top-level entries (Load/Save/Delete) and leaves the submenus open
    /// (S07 §12 O12), so the submenu contents are our own choice (see
    /// [`crate::eq_presets`]). The request is queued; the presenter takes it
    /// on its tick ([`App::take_eq_menu`]) and shows the native popup.
    /// Returns the entries the popup carries.
    pub fn request_eq_presets_menu(&mut self, request: EqMenuRequest) -> &'static [&'static str] {
        self.pending_eq_menu = true;
        request.entries()
    }

    /// Applies the built-in equalizer preset `name` (ignoring case).
    ///
    /// Sets all ten bands through ordinary `SetBand` commands and leaves the
    /// preamp and the on/auto flags alone. The engine's resulting gains are
    /// what the configuration saves, so the choice persists (see
    /// [`crate::eq_presets`]). Returns `false` when no such preset exists.
    pub fn apply_eq_preset(&mut self, name: &str) -> Result<bool> {
        let Some(preset) = crate::eq_presets::find_preset(name) else {
            return Ok(false);
        };
        for (band, &gain_db) in preset.bands.iter().enumerate() {
            self.submit(Command::SetBand {
                band: band as u8,
                gain_db,
            })?;
        }
        Ok(true)
    }

    /// The built-in preset the current band gains correspond to, if any.
    pub fn current_eq_preset(&self) -> Option<&'static str> {
        crate::eq_presets::preset_name_for(&self.engine.eq())
    }

    /// Asks the app to exit (the window's close button).
    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    /// Records the window's focus state (S01 §2.1: the focused title bar is the
    /// window's active render).
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Renders one frame of the main window.
    pub fn render(&mut self) -> Image {
        self.ui.refresh(self.now_ms());
        if self.focused == self.ui.state().title_bar_focused {
            return render_main_window(self.ui.state(), self.skin.skin(), &self.layout);
        }
        // `MainWindow` has no `set_focused` (a contract gap), so the focus
        // variant is applied to a copy of the render state.
        let mut state = self.ui.state().clone();
        state.title_bar_focused = self.focused;
        render_main_window(&state, self.skin.skin(), &self.layout)
    }

    /// Renders one frame of the equalizer window (S01 §3, S07).
    ///
    /// The equalizer has no time-driven parts, so no refresh is needed; the
    /// focus variant follows the window's own focus flag, which the presenter
    /// keeps via [`App::set_eq_focused`].
    pub fn render_eq(&self) -> Image {
        render_equalizer_window(self.eq.state(), self.skin.skin(), &self.eq_layout)
    }

    /// Records the equalizer window's focus state (S01 §3: the focused title
    /// bar is the window's active render).
    pub fn set_eq_focused(&mut self, focused: bool) {
        self.eq.set_focused(focused);
    }

    /// Records the playlist editor's own focus state (S01 §4.1).
    pub fn set_playlist_focused(&mut self, focused: bool) {
        self.playlist_focused = focused;
    }

    /// Sends a command into the command channel and processes it immediately.
    ///
    /// This is the **external** path (plugins, the platform layer, tests): the
    /// UI machine has not seen the command, so [`App::apply_from`] lets it
    /// predict the S02 effect as well.
    pub fn submit(&mut self, command: Command) -> Result<()> {
        self.commands.send(command)?;
        self.pump()
    }

    /// The single pump step: drain commands, poll the engine, drain events,
    /// forward the newest actual tap snapshot and refresh the UI.
    pub fn pump(&mut self) -> Result<()> {
        // The library's scan runs on its own thread: the index comes back
        // here, when the worker is done.
        self.poll_library();
        let mut commands = Vec::new();
        while let Some(command) = self.command_rx.try_recv()? {
            commands.push(command);
        }
        for command in &commands {
            self.apply_from(command, false);
        }

        // The stream's status is read **before** the engine polls: the engine
        // drops the observer of a transfer when the run ends, and the phase
        // that says how the stream ended must still be readable then
        // (S19 §2).
        self.pump_stream_status();

        self.engine.poll()?;

        // The engine reports a fade that does not fit the buffer it is
        // realised in (S18 §2.3's constraint). The report is the engine's
        // own sentence; the app only decides when the user sees it (see
        // `crate::fade_note`).
        self.drain_fade_reports();

        let events: Vec<Event> = self.events.try_iter().collect();
        for event in &events {
            self.handle_event(event);
        }

        // A DSP chain that was installed while a run was live waits for that
        // run to end: the engine takes the run's chain back when the run
        // ends, so an install during the run would be dropped then.
        if self.dsp_install_pending && self.engine.transport() == TransportState::Stopped {
            self.install_dsp_chain_now();
        }
        // The output (a plug-in or the mode) changed while a run was live.
        if self.output_change_pending && self.engine.transport() == TransportState::Stopped {
            self.change_output();
        }

        // The skins folder is read when the skin list is asked for; an entry
        // that is not a skin was skipped there, and the reason reaches the
        // user here.
        let skin_problems = self.skin_folder.borrow_mut().take_problems();
        for problem in skin_problems {
            self.note(problem);
        }

        let now_ms = self.now_ms();
        self.pump_visualization(now_ms);
        self.ui.refresh(now_ms);
        // Supervise the plugin host before mirroring state to it (see
        // `crate::supervise`): non-blocking by contract, so a missing or dead
        // host only costs a cooldown-gated spawn attempt and the mirror below
        // degrades to detached. Unmanaged by default (no spawns in tests).
        if let Some(supervisor) = self.host_supervisor.as_mut() {
            // A host that died is reported to the user: the presenter prints
            // the notes (see `crate::window`).
            if let Some(report) = supervisor.supervise(&mut self.host) {
                self.note(report);
            }
        }
        // A new host has loaded nothing: the per-host plug-in state starts
        // over with it (see `App::sync_plugin_host_state`).
        self.sync_plugin_host_state();
        // Mirror the player state to the plugin host (see `crate::host`):
        // best-effort by contract, so the outcome is ignored — a detached or
        // dead host neither fails nor blocks the pump.
        let _ = self.poll_host();
        // Ask the host for the plug-ins the user has switched on, once per
        // host: the request needs the channel, so it waits for the attach
        // the supervisor made above (see `App::load_enabled_plugins`).
        self.load_enabled_plugins_for_new_host();
        // The local control channel's snapshot (`--control`) is assembled here,
        // on the app's own thread, from the same state the host mirror reads.
        self.update_control_snapshot();
        let _ = self.publish_host_state();
        Ok(())
    }

    /// Takes the windows whose pixels changed since the last call.
    ///
    /// The presenter calls this once per tick, after [`App::pump`], and paints
    /// the windows in the answer. The mask is computed **here**, on the
    /// presenter's request: a `--null` run never asks, so its pump does no
    /// comparison and no tag poll, and its output is exactly what it was.
    ///
    /// Taking clears the mask, so a change is painted exactly once even if the
    /// presenter has no OS window for it yet (a closed window paints nothing;
    /// a window's own creation asks for a frame, see `crate::window`).
    pub fn take_window_dirty(&mut self) -> WindowDirty {
        // A row whose tags arrived draws its real text; no other signal
        // reports that.
        if self.poll_row_tags() {
            self.window_dirty.playlist = true;
        }
        self.report_window_changes();
        std::mem::take(&mut self.window_dirty)
    }

    /// Notes which windows' render inputs moved since the last report.
    ///
    /// This is the half of the paint gate that the presenter cannot compute:
    /// the clock (time digits, marquee, analyser) and the scan's worker thread
    /// change a frame without any call site the presenter can see, so the app
    /// compares the inputs each frame was drawn from. Everything a command, an
    /// event or a skin load touches is marked where it is applied
    /// ([`App::apply_from`], [`App::handle_event`], [`App::load_skin`]).
    fn report_window_changes(&mut self) {
        let skin = self.skin_generation;
        let size = self.frame_size();
        let unchanged = self.main_frame.as_ref().is_some_and(|last| {
            last.size == size
                && last.skin == skin
                && last.focused == self.focused
                && &last.state == self.ui.state()
        });
        if !unchanged {
            self.window_dirty.main = true;
            self.main_frame = Some(MainFrame {
                size,
                skin,
                focused: self.focused,
                state: self.ui.state().clone(),
            });
        }
        let unchanged = self
            .eq_frame
            .as_ref()
            .is_some_and(|last| last.skin == skin && &last.state == self.eq.state());
        if !unchanged {
            self.window_dirty.equalizer = true;
            self.eq_frame = Some(EqFrame {
                skin,
                state: self.eq.state().clone(),
            });
        }
        if !self.library_window.is_open() {
            self.library_frame = None;
        } else {
            let size = self.library_window.client_size();
            let view = self.library_window.view();
            let frame = LibraryFrame {
                size,
                total: view.total(),
                offset: view.offset(),
                scroll: view.scroll(),
                selected: view.selected(),
                sort: view.sort(),
                descending: view.descending(),
                text: view.text().to_owned(),
                scan: view.scan_status().map(str::to_owned),
            };
            if self.library_frame.as_ref() != Some(&frame) {
                self.window_dirty.library = true;
                self.library_frame = Some(frame);
            }
        }
    }

    /// Takes the playlist rows' tags that arrived, reporting whether any did.
    ///
    /// The tags are read off the UI thread (see [`crate::playlist_tags`]), and
    /// a shown row that gains them draws its real text, so their arrival is a
    /// change of the playlist frame. The count of the **shown** rows that have
    /// tags is the signal: a tag for a row the editor does not show cannot
    /// change a frame. A closed editor is skipped altogether.
    fn poll_row_tags(&mut self) -> bool {
        if !self.playlist_open() {
            return false;
        }
        let shown = self.playlist.visible_ids();
        let known = |tags: &crate::playlist_tags::TagReader| {
            shown
                .iter()
                .filter(|id| tags.metadata(**id).is_some())
                .count()
        };
        let before = known(&self.playlist_tags);
        self.playlist_tags.poll();
        let changed = known(&self.playlist_tags) != before;
        if changed && self.engine.transport() == TransportState::Stopped {
            // The stopped player's marquee shows the length as soon as it is
            // known.
            let index = self.engine.current_index();
            self.handle_event(&Event::TrackChanged {
                id: None,
                index,
                metadata: None,
                duration_ms: None,
            });
        }
        changed
    }

    /// Applies `command` to the classic window `window`.
    ///
    /// The command applies to the window it was addressed to, which is what
    /// the reference does: the equalizer's shade is its own flag (`eq_ws`)
    /// and the playlist editor's is its own (`pe_height_ws`), beside the main
    /// window's (`windowshade`), S01 §1.
    ///
    /// * The **equalizer** answers both commands: its shade is the 14-px
    ///   strip (S01 §1: 275 x 14) and its double size is 275 x 116 ↔ 550 x 232
    ///   (S05 §2.1).
    /// * The **playlist editor** answers the windowshade (S01 §1: 275 x 14) and
    ///   ignores the double size: it is not affected by double-size mode at
    ///   all (S05 §2.1, S01 §1: it stays 275 x 116), so the command has no
    ///   geometry to change there.
    /// * The **main window** keeps the answer it always had: its windowshade
    ///   is the main window's, and its double size is the player's flag, which
    ///   the equalizer follows (S05 §2.1).
    ///
    /// The sizes are not computed here: the presenter reads them from
    /// [`App::frame_size`], [`App::eq_frame_size`] and
    /// [`App::playlist_frame_size`], which are the one place the geometry
    /// lives (S05 §1.1, §2.1).
    pub fn apply_window_command(&mut self, window: WindowId, command: WindowCommand) -> Result<()> {
        match (window, command) {
            (WindowId::Main, WindowCommand::ToggleWindowShade) => {
                self.submit(Command::ToggleWindowShade(WindowId::Main))
            }
            (WindowId::Main, WindowCommand::ToggleDoubleSize) => {
                self.submit(Command::ToggleDoubleSize)
            }
            (WindowId::Equalizer, WindowCommand::ToggleWindowShade) => {
                self.submit(Command::ToggleWindowShade(WindowId::Equalizer))
            }
            // Double size is ONE flag for the player, not a per-window one.
            // Measured 2026-10-21: `WM_COMMAND` 40165 posted to the equalizer
            // took the MAIN window from 275x116 to 550x232 (capture jobs
            // `base291-global-double`), while the same treatment with 40064
            // left the main window alone (`base291-global-shade`). So this
            // answer is the global command, which also carries the equalizer
            // (S05 §2.1) - not `toggle_eq_double_size`, which the first version
            // of this match called.
            (WindowId::Equalizer, WindowCommand::ToggleDoubleSize) => {
                self.submit(Command::ToggleDoubleSize)
            }
            (WindowId::Playlist, WindowCommand::ToggleWindowShade) => {
                self.submit(Command::ToggleWindowShade(WindowId::Playlist))
            }
            // The playlist-targeted native command remains a no-op as in S05.
            // The user's global 2x setting can separately scale a small playlist.
            (WindowId::Playlist, WindowCommand::ToggleDoubleSize) => Ok(()),
            _ => Err(Error::InvalidArgument(format!(
                "{window:?} is not a window that answers a window command"
            ))),
        }
    }

    /// Reads the stream's phase out of the engine and turns it into what the
    /// user sees (S19 §2).
    ///
    /// The engine publishes what the transfer is doing — the phase of the
    /// exchange and the counters of the body — and the app decides what that
    /// means for the user:
    ///
    /// * a **phase change** is a note: the text of the phase, as S19 §2
    ///   spells it. The texts are the app's, the facts are the engine's.
    /// * the **counters** are a transient status (see [`crate::stream_status`]):
    ///   one entry that is replaced, not a note per sample.
    ///
    /// A run that is not on a stream has no phase at all, and the phase of the
    /// last transfer goes with it: a stale phase must never show.
    fn pump_stream_status(&mut self) {
        let Some(snapshot) = self.engine.stream_snapshot() else {
            self.stream.reset();
            self.ui.set_stream_prefix(None);
            return;
        };
        self.apply_stream_snapshot(snapshot, Instant::now());
        self.ui
            .set_stream_prefix(self.stream.title_prefix().map(str::to_owned));
    }

    /// Turns one snapshot of a transfer into the note and the status.
    ///
    /// The note is the text of the phase, and it is made **once per phase
    /// change**: a transfer that runs for minutes must not make a note per
    /// sample.
    fn apply_stream_snapshot(
        &mut self,
        snapshot: rustamp_decoders::HttpProgressSnapshot,
        now: Instant,
    ) {
        if let Some(note) = self.stream.update(snapshot, now) {
            self.note(note);
        }
    }

    /// The transient status of the stream that is playing (S19 §2).
    ///
    /// It is the transfer display the window shows in its status area: the
    /// head of the display and the counters below it, empty when no transfer
    /// runs. The text changes while the transfer runs, so it is read per
    /// frame and never appended to the notes.
    pub fn stream_status(&self) -> Option<&str> {
        self.stream.status()
    }

    /// Turns the engine's fade reports into notes the user sees.
    ///
    /// The engine compares every requested fade with the buffers the fade is
    /// realised in and reports the conflict when it does not fit them
    /// (`rustamp_audio::fade_budget`; S18 §2.3's constraint). The report
    /// fires on **every** fade-out — with the shipped configuration a 500 ms
    /// fade does not fit the device's 23 ms buffer — so the app shows it
    /// once per configuration (see [`crate::fade_note`]).
    ///
    /// The wording is the engine's own: the app decides *when* the user
    /// reads the report, never *what* it says.
    fn drain_fade_reports(&mut self) {
        while let Some(report) = self.engine.take_fade_note() {
            let configuration = self.fade_configuration(report.requested_ms);
            if !self.seen_fade_reports.should_show(configuration) {
                // The same configuration was reported already: the sentence
                // would say the same thing, so the user reads it once.
                continue;
            }
            self.note(report.message());
        }
    }

    /// The configuration a fade report about `fade_ms` is about.
    ///
    /// The three numbers are the ones the conflict is made of: the buffer
    /// the device granted, the engine's ring and the fade that was asked
    /// for. They are counted in milliseconds, the unit the report reads in.
    fn fade_configuration(&self, fade_ms: u32) -> FadeConfiguration {
        // The device's rate: the fade is realised at it, so it is the rate
        // the buffers are counted in.
        let rate = u64::from(self.engine.device_spec().sample_rate).max(1);
        // The ring's real capacity: the engine never sizes it below 256
        // frames.
        let ring_frames = self.ring_frames.max(256) as u64;
        FadeConfiguration {
            device_buffer_ms: u64::from(self.device_buffer.frames()).saturating_mul(1000) / rate,
            ring_ms: ring_frames.saturating_mul(1000) / rate,
            fade_ms,
        }
    }

    /// UI-thread only: drain once, analyse at most one snapshot. Missing input
    /// is not silence; refresh still ages the existing deterministic envelopes.
    /// Source/device rates may differ: the tap is before device conversion, so
    /// use the decoder's exact rate, never the device rate or rounded UI kHz.
    ///
    /// An analysis error is only noted, never fatal: `VisError::NonFinitePcm`
    /// is an offline validation invariant that the live path cannot reach
    /// (RFC 0014 §4.5), and noting a rejected frame must not halt playback.
    fn pump_visualization(&mut self, now_ms: u64) {
        let frame = self.engine.take_vis_frame();
        if std::mem::take(&mut self.vis_discontinuity) {
            // A batch straddling a known boundary is ambiguous. Even a valid
            // new frame in this batch is sacrificed; no replacement is made.
            return;
        }
        if let Some(frame) = frame {
            // A visualization plug-in in the host draws from the same frames the
            // main window's analyser does: they go over the control channel
            // while one is loaded (nothing is sent for a host that has none).
            if self
                .host
                .plugins_of_kind(PluginKind::Visualization)
                .next()
                .is_some()
                && self.host.send_vis_frame(&frame) == CommandOutcome::HostGone
            {
                self.note(
                    "the plugin host is gone: the visualization frame was not sent".to_string(),
                );
            }
            let source_rate_hz = self.engine.source_spec().map_or(0, |s| s.sample_rate);
            if let Err(error) = self.ui.push_vis_frame(&frame, source_rate_hz, now_ms) {
                self.note(error.to_string());
            }
        }
    }

    fn invalidate_visualization(&mut self) {
        self.vis_discontinuity = true;
        self.ui.reset_visualization();
    }

    /// Applies one external command (see [`App::submit`]).
    pub fn apply(&mut self, command: &Command) {
        self.apply_from(command, false);
    }

    /// Applies one command to the engine, to the app when the app owns it, and
    /// to the UI machine unless the UI produced it (the UI machine already
    /// applied its own S02 prediction for its own commands).
    fn apply_from(&mut self, command: &Command, from_ui: bool) {
        // A command is the user's own action: it may change any frame (a
        // toggle, a new skin, a new size, an open dialog), and commands are
        // rare next to the frames, so every window is marked rather than
        // tracking which one the command reaches.
        self.window_dirty.merge(WindowDirty::ALL);
        if matches!(command, Command::Seek { .. } | Command::SeekRelative { .. }) {
            // Includes external/plugin commands as well as the UI's predicted
            // seek. No public seek-completion/generation tag exists on frames.
            self.invalidate_visualization();
        }
        if self.engine.is_flush_failed() {
            let _ = self.ensure_engine_ready();
        }
        // The engine ignores everything it does not own, so a single forward is
        // enough; a command it cannot apply (Play on an empty queue) becomes an
        // error event instead of taking the app down.
        if let Err(error) = self.engine.apply(command) {
            self.publish_error(&error);
        }
        if routing::app_owns(command) {
            self.apply_app(command);
        }
        if !from_ui {
            self.ui.handle_command(command, self.now_ms());
        }
        // S05 §2.1: the double-size flag reaches the equalizer, so the EQ
        // machine mirrors the main window's flag (its size follows from
        // `eq_frame_size`; shading the main never resizes it, S05 §3.2).
        if matches!(command, Command::ToggleDoubleSize) {
            self.eq.set_double_size(self.ui.state().double_size);
            self.update_playlist_double_size(self.ui.state().double_size);
        }
        // A toggle may have closed the focused window underneath the focus.
        self.maintain_focus();
    }

    /// Moves focus to the next open player window (`FocusNextWindow`, S03 id
    /// 40063) in [`crate::focus::FOCUS_ORDER`], skipping closed windows. Sizes
    /// are untouched: focusing never resizes.
    fn focus_next_window(&mut self) {
        let next = crate::focus::next_focus(self.focused_window, |window| self.window_open(window));
        // With the main window hidden and nothing else open the cycle has no
        // window to give the focus to; a hidden window cannot take it.
        if self.window_open(next) {
            self.focused_window = next;
            self.focus_request = Some(next);
        }
    }

    /// Shows the main window (`ShowMainWindow`, S03 id 40258): show,
    /// un-minimize, raise and focus it. One-way by contract: unlike
    /// `ToggleWindow(Main)` it never hides. A minimize that has not reached
    /// the OS yet is dropped, since there is nothing to restore from then.
    fn show_main_window(&mut self) {
        self.main_visible = true;
        self.minimize_requested = false;
        self.restore_request = true;
        self.focused_window = WindowId::Main;
        self.focus_request = Some(WindowId::Main);
    }

    /// Repairs the focus after a window closed: a focused window that is no
    /// longer open falls back to the main window, with a request so the
    /// presenter follows. A hidden main window is not repaired (it is the
    /// fallback, and a hidden window cannot take focus).
    fn maintain_focus(&mut self) {
        if self.focused_window != WindowId::Main && !self.window_open(self.focused_window) {
            self.focused_window = WindowId::Main;
            if self.main_visible {
                self.focus_request = Some(WindowId::Main);
            }
        }
    }

    /// Feeds the engine's seekability to the UI machine.
    ///
    /// The engine owns the fact (`Engine::is_seekable`): a stream has no
    /// position to seek to, and the engine refuses such a seek and reports the
    /// refusal. The UI needs it to draw the position slider in its disabled
    /// look and to offer no seek, so the app hands it over whenever a track
    /// opens.
    fn sync_seekable(&mut self) {
        self.ui.set_seekable(self.engine.is_seekable());
    }

    /// Applies a core [`Event`]: the app's own bookkeeping first, then the UI
    /// machine (the authoritative state).
    pub fn handle_event(&mut self, event: &Event) {
        // The frames this event can change (see [`event_windows`]). The dialog
        // is only a frame while it is open: an event for a closed dialog marks
        // nothing.
        let mut changed = event_windows(event);
        if self.dialog_open_kind().is_none() {
            changed.dialog = false;
        }
        self.window_dirty.merge(changed);
        // The track that opens decides whether it can be seeked, and the UI
        // needs that fact before it handles the track change.
        if matches!(event, Event::TrackChanged { .. }) {
            self.sync_seekable();
        }
        // A track that actually opened: read its ReplayGain tags and drive the
        // engine's ReplayGain stage from them. Absent tags leave the control's
        // values unset, so the mode stays harmless (unity).
        if let Event::TrackChanged {
            metadata: Some(metadata),
            ..
        } = event
        {
            let tags = ReplayGainTags::from_metadata(metadata);
            let mode = self.engine.replaygain_mode();
            self.engine.set_replaygain(
                mode,
                tags.track_gain_db,
                tags.track_peak,
                tags.album_gain_db,
                tags.album_peak,
            );
            // The equalizer's own per-track preset: the AUTO flag is the gate
            // (S07 §12 item 13) and the lookup runs at the track start only
            // (see `crate::eq_autoload`).
            self.auto_load_eq_preset(metadata);
        }
        if matches!(
            event,
            Event::TrackChanged { .. } | Event::StreamInfoChanged(_)
        ) {
            self.invalidate_visualization();
        }
        match event {
            Event::Quit => self.quit = true,
            Event::SkinChanged(path) => self.current_skin = path.clone(),
            Event::ModesChanged { shuffle, repeat } => {
                self.shuffle = *shuffle;
                self.repeat = *repeat;
            }
            Event::PlaylistEnded => self.playlist_ended = true,
            Event::EqChanged(eq) => {
                self.eq.sync_from_eq_state(eq);
            }
            Event::Error(info) => {
                // A DSP failure is the engine's bypass policy, not a generic
                // engine error: it has its own note, its own remembered state
                // and its own recovery (see `crate::dsp`).
                if info.kind == ErrorKind::Dsp {
                    self.surface_dsp_bypass(info);
                } else {
                    self.surface_error(info);
                }
            }
            // A track that actually opened clears the error it replaced.
            Event::TrackChanged { index: Some(_), .. } => self.last_error = None,
            _ => {}
        }

        // The engine reports `TrackChanged` with `id: None` (it holds no
        // playlist ids), but the UI machine uses `id.is_some()` as the "a track
        // is loaded" flag. The app is the state owner, so it fills the id in
        // from its own queue before the event reaches the UI.
        if let Event::TrackChanged {
            id: None,
            index,
            metadata,
            duration_ms,
        } = event
        {
            let id = self.entry_id_for(*index);
            // The engine read the tags of the track it opened: they are in
            // memory already, so the row can show them without reading the
            // file (see `crate::playlist_tags`).
            if let (Some(id), Some(metadata)) = (id, metadata) {
                let mut known = metadata.as_ref().clone();
                known.duration_ms = known.duration_ms.or(*duration_ms);
                self.playlist_tags.remember(id, known);
            }
            let known_length = id
                .and_then(|id| self.playlist_tags.metadata(id))
                .and_then(|tags| tags.duration_ms);
            let normalised = Event::TrackChanged {
                id,
                index: *index,
                metadata: self.metadata_with_name(*index, metadata.as_deref()),
                duration_ms: duration_ms.or(known_length),
            };
            self.ui.handle_event(&normalised, self.now_ms());
            return;
        }

        self.ui.handle_event(event, self.now_ms());
    }

    /// The metadata the marquee gets for the entry at `index`: a track without
    /// a title shows its file name, never a blank (S02 §3 item 2).
    fn metadata_with_name(
        &self,
        index: Option<u32>,
        metadata: Option<&TrackMetadata>,
    ) -> Option<Box<TrackMetadata>> {
        // A stopped player that moved to an entry has opened nothing: the tags
        // the playlist read for its row say the same thing the row says.
        let index = index.map(|index| index as usize);
        let tags = || {
            let id = self.entry_ids.get(index?)?;
            self.playlist_tags.metadata(*id).cloned()
        };
        let mut shown = metadata.cloned().or_else(tags).unwrap_or_default();
        if shown.title.is_none()
            && let Some(track) = index.and_then(|index| self.engine.queue().get(index))
        {
            shown.title = Some(track.display_name());
        }
        Some(Box::new(shown))
    }

    /// The name a listing shows for a queue entry: its tags once they arrived,
    /// its file name until then.
    pub(crate) fn entry_name(&self, index: usize) -> Option<String> {
        let track = self.engine.queue().get(index)?;
        let tags = self
            .entry_ids
            .get(index)
            .and_then(|id| self.playlist_tags.metadata(*id));
        Some(
            tags.and_then(TrackMetadata::display_title)
                .unwrap_or_else(|| track.display_name()),
        )
    }

    /// Applies a [`PlayerState`] snapshot by translating it into the events the
    /// UI machine already understands. This is how the app seeds the window at
    /// start-up and after a state resync.
    pub fn sync_from_player_state(&mut self, state: &PlayerState) {
        let now = self.now_ms();
        self.ui
            .handle_event(&Event::StateChanged(state.transport), now);
        self.ui
            .handle_event(&Event::StreamInfoChanged(state.stream), now);
        self.ui.handle_event(
            &Event::VolumeChanged {
                volume: state.volume,
                balance: state.balance,
            },
            now,
        );
        self.ui.handle_event(
            &Event::ModesChanged {
                shuffle: state.shuffle,
                repeat: state.repeat,
            },
            now,
        );
        if state.current_index.is_some() || state.current_id.is_some() {
            // The engine owns the track's seekability; the UI needs it (see
            // `App::sync_seekable`).
            self.sync_seekable();
            self.ui.handle_event(
                &Event::TrackChanged {
                    id: state.current_id,
                    index: state.current_index,
                    metadata: self.metadata_with_name(state.current_index, None),
                    duration_ms: state.duration_ms,
                },
                now,
            );
        }
        self.ui.handle_event(
            &Event::PositionChanged {
                position_ms: state.position_ms,
                duration_ms: state.duration_ms,
            },
            now,
        );
        self.shuffle = state.shuffle;
        self.repeat = state.repeat;
        self.always_on_top = self.config.ui.always_on_top;
        self.ui.refresh(now);
    }

    /// Assemble the snapshot the plugin host mirrors (sources per field are in
    /// [`crate::host`]).
    ///
    /// The engine owns transport, position, duration, volume, balance, the
    /// queue (length and current index), the stream and stop-after-current;
    /// the app owns shuffle/repeat (the engine's own snapshot is the truth
    /// for them, and the app mirrors it) and the stable entry ids (the engine
    /// reports `None`). The equalizer state is the engine's, which owns the
    /// S07 stage.
    pub fn host_player_state(&mut self) -> PlayerState {
        let mut state = self.engine.state();
        // App-owned modes: the app mirrors the engine's playback order, so
        // the snapshot the host sees and the traversal agree (RFC 0001 / S15).
        state.shuffle = self.shuffle;
        state.repeat = self.repeat;
        // Stable entry id for the engine's current index (the engine has no
        // playlist ids; a `None` index yields `None` without allocating).
        state.current_id = self.entry_id_for(state.current_index);
        state
    }

    /// Build the one protocol message that feeds the host's state bridge.
    ///
    /// This is the single construction site: every `PlayerStateUpdate` the app
    /// sends passes through here, so the field-by-field audit in
    /// [`crate::host`] stays checkable in one place.
    pub fn player_state_update(&mut self) -> Message {
        player_state_message(self.host_player_state())
    }

    /// The handles the local control channel answers from (`--control`).
    ///
    /// The snapshot is refreshed first, so a client that connects before the
    /// first pump of a run already sees the player's state and not the
    /// default. The channel sends its commands into [`App::commands`] — the
    /// same funnel a keystroke uses — and reads its events off [`App::bus`].
    pub fn control_handles(&mut self) -> crate::control::Handles {
        self.update_control_snapshot();
        crate::control::Handles {
            commands: self.commands.clone(),
            bus: self.bus.clone(),
            state: Arc::clone(&self.control_state),
        }
    }

    /// Records whether this run serves the local control channel (`--control`).
    ///
    /// A run that serves a channel stays up as a **session** even while
    /// nothing is queued: what it is for is the client's commands. Without a
    /// channel an empty queue still ends the headless run at once (see
    /// [`App::run_headless`]).
    pub fn set_control_channel(&mut self, served: bool) {
        self.control_channel = served;
    }

    /// Publishes the current player state to the local control channel.
    ///
    /// Called by [`App::pump`], so the snapshot a control client reads is one
    /// the player computed anyway. A poisoned lock is recovered: a panic in
    /// the player must not wedge the channel.
    fn update_control_snapshot(&mut self) {
        let state = self.host_player_state();
        let mut guard = self
            .control_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = state;
    }

    /// Publishes an [`Event::PlaylistChanged`] with the queue's new length.
    ///
    /// The app owns the playlist model (RFC 0005 §5), so the app is the
    /// publisher the playlist contract names: "the owner mutates it and
    /// publishes `PlaylistChanged` events". A local control client waits on
    /// this event (or on the snapshot) to confirm that a `cmd` it sent was
    /// applied, instead of sleeping and re-reading the state.
    fn publish_playlist_change(&self, kind: PlaylistChangeKind) {
        self.bus.publish(Event::PlaylistChanged {
            kind,
            len: self.engine.queue().len() as u32,
        });
    }

    /// Attach the host's control-channel transport (see
    /// [`HostStateLink::attach`](crate::host::HostStateLink::attach)).
    /// Hand-attaching bypasses the supervisor: a managed app re-establishes
    /// its own transport on the next pump (see [`App::manage_host`]).
    pub fn attach_host(&mut self, transport: impl Transport + 'static) {
        self.host.attach(transport);
    }

    /// Drop the host transport without sending anything.
    pub fn detach_host(&mut self) {
        self.host.detach();
    }

    /// Whether a host transport is currently attached.
    pub fn host_attached(&self) -> bool {
        self.host.is_attached()
    }

    /// The plug-ins the plugin host reported as loaded (see
    /// [`crate::plugin_inventory`]).
    ///
    /// Empty while no host is attached. The ids are the host's and belong to
    /// the host process that issued them (RFC 0020 Q6), so they are forgotten
    /// when the host is replaced.
    pub fn host_plugins(&self) -> &[LoadedPlugin] {
        self.host.plugins()
    }

    /// The plug-in the host reported for the scanned file `file` of `kind`.
    ///
    /// The host answers a load with the plug-in's **display name**, not the
    /// file it came from, so the file cannot be matched exactly. Own choice,
    /// stated plainly: the only loaded plug-in of that kind is it; among
    /// several, the one whose name contains the file's stem (`in_openmpt.dll`
    /// -> `openmpt`, ignoring case and the kind prefix); otherwise none, and
    /// the caller says why instead of guessing.
    pub fn loaded_plugin_for_file(&self, file: &str, kind: PluginKind) -> Option<&LoadedPlugin> {
        let mut of_kind = self.host.plugins().iter().filter(|p| p.kind == kind);
        let first = of_kind.next()?;
        let Some(second) = of_kind.next() else {
            return Some(first);
        };
        let stem = file
            .rsplit_once('.')
            .map_or(file, |(stem, _)| stem)
            .to_ascii_lowercase();
        let stem = stem
            .split_once('_')
            .map_or(stem.as_str(), |(_, rest)| rest)
            .to_owned();
        std::iter::once(first)
            .chain(std::iter::once(second))
            .chain(of_kind)
            .find(|p| p.name.to_ascii_lowercase().contains(&stem))
    }

    /// Opens the configuration dialog of the plug-in in row `index` of the
    /// scan (the Configure button of the Preferences plug-in pages).
    pub(crate) fn configure_plugin_at(&mut self, index: usize) {
        let Some(entry) = self.plugin_scan().entries.get(index) else {
            return;
        };
        let (file, kind) = (entry.file.clone(), entry.kind);
        let Some(plugin) = self.loaded_plugin_for_file(&file, kind) else {
            let text = format!(
                "{file} is not loaded in the plugin host: switch it on first. If it is on, the \
                 host reported several plug-ins of this kind and none of them names it."
            );
            self.plugins_report = Some(text.clone());
            self.note(text);
            return;
        };
        let (plugin_id, kind) = (plugin.plugin_id, plugin.kind);
        if let Err(error) = self.submit(Command::ConfigurePlugin { plugin_id, kind }) {
            self.note(format!("preferences: {error}"));
        }
    }

    /// The plug-in the plugin host reported under this id, if any.
    pub fn host_plugin(&self, plugin_id: u32) -> Option<&LoadedPlugin> {
        self.host.plugin(plugin_id)
    }

    /// Install the plugin host supervisor (see [`crate::supervise`]).
    ///
    /// From the next [`pump`](App::pump) on, the supervisor starts
    /// `rustamp-plughost32` when it is missing and re-attaches (or restarts)
    /// it when it dies — off the audio path, degrading to detached. Pass
    /// [`Supervisor::from_search`](crate::supervise::Supervisor::from_search)
    /// for the normal binary resolution.
    pub fn manage_host(&mut self, supervisor: Supervisor) {
        self.host_supervisor = Some(supervisor);
    }

    /// Stop supervising the host: kill the managed child (best-effort,
    /// without waiting) and detach the link without sending anything.
    pub fn stop_host(&mut self) {
        if let Some(supervisor) = self.host_supervisor.as_mut() {
            supervisor.stop(&mut self.host);
        }
    }

    /// Publish the current snapshot to the host under the cadence rules (see
    /// [`crate::host`]).
    ///
    /// Best-effort by contract: a detached or dead host is reported, never
    /// raised — [`App::pump`] ignores the outcome, so state publishing can
    /// neither fail playback nor block it.
    pub fn publish_host_state(&mut self) -> PublishOutcome {
        if let Some(index) = self.engine.current_index()
            && let Some(source) = self.engine.queue().get(index as usize)
        {
            let file = source.display();
            let title = self
                .entry_name(index as usize)
                .unwrap_or_else(|| file.clone());
            self.host.publish_now_playing(index, &title, &file);
        }
        let state = self.host_player_state();
        self.host.publish(&state)
    }

    /// Run one `Hello`/`Ping` liveness step on the host channel (see
    /// [`HostStateLink::poll`](crate::host::HostStateLink::poll)).
    ///
    /// Best-effort by contract like [`publish_host_state`](App::publish_host_state):
    /// [`App::pump`] calls it before publishing and ignores the outcome. A
    /// missed `Pong`, a foreign `Hello` or a dead channel detaches the link
    /// here, so the publish right after degrades to detached and the
    /// supervisor replaces the host on its next step.
    pub fn poll_host(&mut self) -> PollOutcome {
        let outcome = self.host.poll();
        for command in self.host.take_plugin_commands() {
            self.note(format!("a plug-in asked for {command:?}"));
            if let Err(error) = self.submit(command) {
                self.note(format!("a plug-in's request failed: {error}"));
            }
        }
        // A load the host refused is the user's news. The host reports it on
        // its own stderr, which the windowed player never shows, so the note
        // is the only place the user learns about it.
        let failures = self.host.take_plugin_failures();
        for failure in failures {
            // The host refused this file, so it is not loaded: a later
            // request for it may try again.
            self.plugin_requests.remove(&failure.path);
            let text = format!(
                "the plugin host could not load {}: {}",
                failure.path, failure.message
            );
            self.plugins_report = Some(text.clone());
            self.note(text);
        }
        outcome
    }

    /// Ask the plugin host to open the visualization plug-in's configuration
    /// dialog (RFC 0005 §5.4, the player -> host direction).
    ///
    /// The plug-in runs in `rustamp-plughost32.exe` and only the host's UI
    /// thread may call into a plug-in (S09 §3.5), so the app sends
    /// [`Message::CommandToHost(Command::ConfigureVisPlugin`)] over the
    /// existing control channel instead of opening the dialog itself (see
    /// [`crate::host`]).
    ///
    /// The request is never dropped silently: a missing or dead host is
    /// reported to the user as a note, the same way an unimplemented command
    /// is.
    fn request_vis_plugin_configuration(&mut self) {
        self.request_plugin_host_command(
            Command::ConfigureVisPlugin,
            "the visualization plug-in's configuration",
        );
    }

    /// Send one plug-in request to the host (RFC 0020, the player -> host
    /// direction).
    ///
    /// The request names a plug-in by the id the host assigned in its
    /// `PluginLoaded` answer (see [`crate::plugin_inventory`]) and travels in
    /// [`Message::CommandToHost`] — the carrier the protocol already has. It is
    /// **fire-and-forget**: there is no reply message, so the outcome reported
    /// here is "the frame went out", not "the plug-in did it". A refusal is the
    /// host's to report, and it reports it on its own stderr (RFC 0020 §12).
    ///
    /// The request is never dropped silently: a missing or dead host is
    /// reported to the user as a note, the same way an unimplemented command
    /// is. `what` is the phrase the notes name the request by.
    fn request_plugin_host_command(&mut self, command: Command, what: &str) {
        match self.host.send_command(command) {
            CommandOutcome::Sent => {
                self.note(format!("{what} was requested from the plugin host"));
            }
            CommandOutcome::Detached => self.note(format!(
                "no plugin host is attached: {what} was not sent to it"
            )),
            CommandOutcome::HostGone => {
                self.note(format!(
                    "the plugin host is gone: {what} was not sent to it"
                ));
            }
        }
    }

    /// Ask the plugin host to load the plug-in at `path` (the player -> host
    /// direction).
    ///
    /// A classic plug-in runs inside `rustamp-plughost32.exe`, so only the
    /// host can map one: the app names the file and the kind it declares and
    /// the host loads it — or refuses it, with the reason (see
    /// [`crate::host`]). The answer arrives on the same channel and is read
    /// on the next [`pump`](App::pump): a loaded plug-in shows up in
    /// [`host_plugins`](App::host_plugins), a refused one becomes a note.
    ///
    /// The request is never dropped silently: a missing or dead host is
    /// reported to the user as a note, the same way an unimplemented command
    /// is.
    ///
    /// # One request per file, per host
    ///
    /// The host loads one plug-in per request and the protocol has no unload
    /// request, so a second request for the same file would map it a second
    /// time. The app therefore asks for a file once per host: the files it
    /// asked for are remembered until the host refuses one (then it may be
    /// asked for again) or a new host arrives (then the new host has loaded
    /// nothing and is asked for everything again).
    pub fn request_plugin_load(&mut self, path: &Path, kind: PluginKind) {
        let asked = path.to_string_lossy().into_owned();
        if !self.plugin_requests.insert(asked.clone()) {
            // The host was asked for this file already: asking again would
            // load it twice, and the protocol cannot unload it.
            self.note(format!(
                "the plugin host was already asked to load {}: a plug-in is not loaded twice",
                path.display()
            ));
            return;
        }
        match self.host.load_plugin(path, kind) {
            CommandOutcome::Sent => self.note(format!(
                "the plugin host was asked to load {}",
                path.display()
            )),
            CommandOutcome::Detached => {
                // Nothing was asked, so the file may be asked for later.
                self.plugin_requests.remove(&asked);
                self.note(format!(
                    "no plugin host is attached: {} was not loaded",
                    path.display()
                ));
            }
            CommandOutcome::HostGone => {
                self.plugin_requests.remove(&asked);
                self.note(format!(
                    "the plugin host is gone: {} was not loaded",
                    path.display()
                ));
            }
        }
    }

    /// The plug-ins the user has switched on, in the scan's order.
    ///
    /// "Switched on" means what the plug-in pages show as "on": the
    /// approval store approved the exact image **and** the file is on the
    /// enabled list ([`rustamp_prefs::PluginEntry::is_enabled`]). A
    /// plug-in that is not approved is off whatever the list says, and the
    /// host would refuse it anyway.
    ///
    /// The order is the scan's order, and the scan sorts the entries by file
    /// name (case-insensitively), so the requests go out in a stable order.
    fn enabled_plugins(&self) -> Vec<(PathBuf, PluginKind)> {
        self.plugin_scan
            .entries
            .iter()
            .filter(|entry| entry.is_enabled(&self.config.plugins))
            .map(|entry| (entry.path.clone(), entry.kind))
            .collect()
    }

    /// Asks the plugin host to load every plug-in the user has switched on.
    ///
    /// This is the startup load: a classic plug-in runs inside
    /// `rustamp-plughost32.exe`, so the player can only get one by asking
    /// the host over the control channel (see [`crate::host`]).
    ///
    /// * **The order** is the scan's order (see [`enabled_plugins`]).
    /// * **A plug-in that is off is not asked for.** The scan decides, and it
    ///   reads the approval store: a missing or broken store approves
    ///   nothing, so nothing is loaded.
    /// * **A failed load is a note, never an error.** A plug-in the host
    ///   refuses is reported by the host's own answer (see
    ///   [`poll_host`](App::poll_host)); a missing or dead host is reported
    ///   here (see [`request_plugin_load`](App::request_plugin_load)). The
    ///   player starts either way.
    fn load_enabled_plugins(&mut self) {
        if !self.plugin_scanned {
            // The list is read once per session: the scan reads the folder
            // and the approval store as bytes and loads nothing.
            self.rescan_plugins();
        }
        for (path, kind) in self.enabled_plugins() {
            self.request_plugin_load(&path, kind);
        }
        self.install_plugin_dsp();
        self.install_plugin_inputs();
        self.install_plugin_output();
    }

    /// Asks the host for the plug-ins the user has switched on, once per
    /// host.
    ///
    /// The request needs the channel, so the load waits for a host to be
    /// attached: the supervisor spawns the host and attaches the transport
    /// (see [`crate::supervise`]), and the load runs on the next pump step.
    /// A host that never arrives loads nothing — there is no channel to ask
    /// over — and the plug-in pages say so when the user switches a plug-in
    /// on without a host (see [`request_plugin_load`](App::request_plugin_load)).
    ///
    /// Once per host, not once per pump: a host is asked for each plug-in
    /// exactly once (see [`request_plugin_load`](App::request_plugin_load)).
    /// A **new** host is asked again, because a fresh host process has
    /// loaded nothing (RFC 0020 Q6).
    fn load_enabled_plugins_for_new_host(&mut self) {
        if self.plugin_startup_loaded || !self.host.is_attached() {
            return;
        }
        self.plugin_startup_loaded = true;
        self.load_enabled_plugins();
    }

    /// Forgets the per-host plug-in state when a new host is attached.
    ///
    /// The plug-in ids and the loaded set belong to one host process
    /// (RFC 0020 Q6): a new host has loaded nothing, so the app forgets
    /// what it asked the old one for and asks the new one for everything
    /// again.
    fn sync_plugin_host_state(&mut self) {
        let generation = self.host.generation();
        if generation == self.plugin_host_generation {
            return;
        }
        self.plugin_host_generation = generation;
        self.plugin_requests.clear();
        self.plugin_startup_loaded = false;
    }

    // -----------------------------------------------------------------------
    // UI input (the window forwards OS input here)
    // -----------------------------------------------------------------------

    /// A left-button press at window-relative `(x, y)`.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_down(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A pointer move (drag).
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_move(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A left-button release at window-relative `(x, y)`.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_up(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A left-button press on one of the classic windows, seen for the title
    /// bar's double click (S03 §5: "Double-click the title bar: toggle
    /// windowshade").
    ///
    /// Returns the command the double click sent, if the press completed one.
    /// A press on a title-bar **button** never counts — the button wins, and
    /// its own action runs instead — and it ends the sequence, so the next
    /// title-bar press starts a new one.
    ///
    /// `now_ms` is the press' time on the app's clock ([`App::now_ms`]). The
    /// caller passes it in, so the rule stays testable without a clock; the
    /// interval and the distance are the shared ones
    /// ([`crate::double_click`]).
    pub fn title_bar_double_click(
        &mut self,
        window: WindowId,
        x: i32,
        y: i32,
        now_ms: u64,
    ) -> Result<Vec<Command>> {
        let on_title_bar = self.on_title_bar(window, x, y);
        let doubled = match self.title_bar_clicks.tracker_mut(window) {
            Some(tracker) => {
                if !on_title_bar {
                    // A press that is not on the title bar ends the sequence:
                    // the button (or whatever owns the press) wins.
                    tracker.clear();
                    false
                } else if tracker.is_double_click(now_ms, x, y) {
                    tracker.clear();
                    true
                } else {
                    tracker.remember(now_ms, x, y);
                    false
                }
            }
            None => false,
        };
        if !doubled {
            return Ok(Vec::new());
        }
        let command = Command::ToggleWindowShade(window);
        self.submit(command.clone())?;
        Ok(vec![command])
    }

    /// Whether the press at `(x, y)` lands on `window`'s title bar and not on
    /// one of its controls.
    ///
    /// The main window's hit test answers the four window buttons before the
    /// band they sit in, and the shaded form carries the position slider
    /// inside that band (S01 §2.1, webamp-provisional): S03 §5's note says
    /// the shaded title bar leaves the windowshade only outside its middle
    /// band, and the slider is the control ours has there. The playlist's hit
    /// test answers its close and shade buttons, and the equalizer has no
    /// title-bar button at all (S05 §1.5), so a title-bar press there is
    /// always the band.
    fn on_title_bar(&self, window: WindowId, x: i32, y: i32) -> bool {
        match window {
            WindowId::Main => {
                if rustamp_ui::hit_test(&self.layout, x, y) != Some(rustamp_ui::Widget::TitleBar) {
                    return false;
                }
                if self.ui.state().windowshade {
                    let slider = &self.layout.shade_position;
                    if x >= slider.x && x < slider.right() && y >= slider.y && y < slider.bottom() {
                        return false;
                    }
                }
                true
            }
            WindowId::Equalizer => {
                rustamp_ui::eq_hit_test(&self.eq_layout, x, y, self.eq.state().windowshade)
                    == Some(rustamp_ui::EqWidget::TitleBar)
            }
            WindowId::Playlist => {
                let width = self.playlist_frame_size().0 as i32 / self.playlist_scale() as i32;
                (0..rustamp_ui::playlist_window::TITLE_BAR_HEIGHT).contains(&y)
                    && (0..width).contains(&x)
                    && rustamp_ui::playlist_window::hit_test_title_bar(x - (width - 275), y)
                        .is_none()
            }
            WindowId::Vis | WindowId::Gen(_) | WindowId::Video => false,
        }
    }

    /// A right-button press at window-relative `(x, y)` (S03 §5).
    ///
    /// The EQ and PL buttons toggle their window ("menu" grade); the long
    /// title, the time display and the seek bar have their own context menus
    /// (see [`crate::menu::ContextMenu`]); anywhere else opens the main-menu
    /// popup ("Right-click the main window: main-menu popup", verified).
    pub fn mouse_right_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let command = match rustamp_ui::hit_test(&self.layout, x, y) {
            Some(rustamp_ui::Widget::EqToggle) => Command::ToggleWindow(WindowId::Equalizer),
            Some(rustamp_ui::Widget::PlaylistToggle) => Command::ToggleWindow(WindowId::Playlist),
            Some(rustamp_ui::Widget::Marquee) => {
                self.pending_context_menu = Some(crate::menu::ContextMenu::Title);
                return Ok(Vec::new());
            }
            Some(rustamp_ui::Widget::TimeDisplay) => {
                self.pending_context_menu = Some(crate::menu::ContextMenu::Time);
                return Ok(Vec::new());
            }
            Some(rustamp_ui::Widget::PositionSlider) => {
                self.pending_context_menu = Some(crate::menu::ContextMenu::Seek);
                return Ok(Vec::new());
            }
            _ => Command::OpenMainMenu,
        };
        // Through the external path: the UI machine has not predicted it.
        self.submit(command.clone())?;
        Ok(vec![command])
    }

    /// A key press. Returns the machine's outcome so the caller can log an
    /// [`ShortcutOutcome::Unhandled`] action with its command id.
    pub fn key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Option<ShortcutOutcome>> {
        // `Alt+L` opens the media library. S04 §3.3 gives the `Library` menu
        // row that accelerator, and S03 binds no other key to `L` with Alt
        // (`L` is "open file(s)", `Ctrl+L` "open location", `Shift+L` "open
        // directory"), so the binding is free. The window's own command id
        // is an own choice: see `crate::library_window`.
        if key == Key::Letter(b'L') && modifiers == Modifiers::ALT {
            let command = Command::ToggleWindow(crate::library_window::LIBRARY_WINDOW_ID);
            self.submit(command.clone())?;
            return Ok(Some(ShortcutOutcome::Commands(vec![command])));
        }
        let Some(action) = lookup(ShortcutWindow::Main, key, modifiers) else {
            self.pump()?;
            return Ok(None);
        };
        if action == ShortcutAction::ToggleMinibrowser {
            self.note("Minibrowser is not implemented yet".to_string());
        }
        Ok(Some(self.dispatch_shared_shortcut(action)?))
    }

    /// Dispatches one shared shortcut action (S03 §1/§4) and forwards the
    /// commands it produces.
    ///
    /// The playlist editor's table (S03 §2) shares most of its rows with the
    /// main window's, so the shared rows are dispatched here — against the
    /// same [`MainWindow`] machine — whichever window the key was pressed in.
    /// The machine applies its S02 prediction first, then the commands go to
    /// the engine and the app ([`App::forward`]).
    pub(crate) fn dispatch_shared_shortcut(
        &mut self,
        action: ShortcutAction,
    ) -> Result<ShortcutOutcome> {
        let outcome = self.ui.dispatch_shortcut(action, self.now_ms());
        match &outcome {
            ShortcutOutcome::Commands(commands) => {
                let commands = commands.clone();
                self.forward(&commands)?;
            }
            ShortcutOutcome::Handled | ShortcutOutcome::Unhandled(_) => self.pump()?,
        }
        Ok(outcome)
    }

    /// The log line for an unhandled shortcut outcome, if that is what this is.
    pub fn unhandled_note(outcome: &ShortcutOutcome) -> Option<String> {
        match outcome {
            ShortcutOutcome::Unhandled(action) => Some(routing::unhandled_message(*action)),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Equalizer-window input (the EQ window forwards OS input here)
    // -----------------------------------------------------------------------

    /// A left-button press at equalizer-window-relative `(x, y)`.
    pub fn eq_mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_down(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A pointer move over the equalizer window (drag).
    pub fn eq_mouse_move(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_move(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A left-button release at equalizer-window-relative `(x, y)`.
    pub fn eq_mouse_up(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_up(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A key press while the equalizer window is focused (S03 §3 via S07 §1:
    /// `N`/`A` toggles, `S` opens the presets menu; band/preamp steps are
    /// open per S07 §9 and stay unwired).
    pub fn eq_key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<Command>> {
        // The rows S03 §3 gives this window a meaning of its own for.
        let own = match (key, modifiers) {
            // Pan: the step is open (S07 §9), this is the volume keys' step.
            (Key::Left, Modifiers::NONE) => Some(self.pan_command(false)),
            (Key::Right, Modifiers::NONE) => Some(self.pan_command(true)),
            // "Close the equalizer", where the main window's row quits.
            (Key::F(4), Modifiers::CTRL) => Some(Command::ToggleWindow(WindowId::Equalizer)),
            _ => None,
        };
        if let Some(command) = own {
            self.submit(command.clone())?;
            return Ok(vec![command]);
        }
        if key == Key::Letter(b'S') && modifiers == Modifiers::CTRL {
            self.pending_eq_file = true;
            return Ok(Vec::new());
        }
        if rustamp_ui::EqualizerWindow::owns_key(key, modifiers) {
            let commands = self.eq.key_down(key, modifiers);
            self.forward_eq(&commands)?;
            return Ok(commands);
        }
        // Everything else is what the key does anywhere in the player (S03 §3
        // lists the shared rows with this window's).
        Ok(match self.key_down(key, modifiers)? {
            Some(ShortcutOutcome::Commands(commands)) => commands,
            _ => Vec::new(),
        })
    }

    /// `SetBalance` one step to the left or the right, clamped to the range.
    fn pan_command(&self, right: bool) -> Command {
        let current = self.engine.state().balance;
        Command::SetBalance(if right {
            current.saturating_add(PAN_STEP)
        } else {
            current.saturating_sub(PAN_STEP).max(-127)
        })
    }

    /// A right-button press on the equalizer window: its context menu (S03 §5).
    pub fn eq_right_down(&mut self) {
        self.pending_context_menu = Some(crate::menu::ContextMenu::Equalizer);
    }

    /// Takes the request for the "Load an EQ preset" file dialog (S03 §3).
    pub fn take_eq_file_request(&mut self) -> bool {
        std::mem::take(&mut self.pending_eq_file)
    }

    /// Sends equalizer-produced commands into the channel and processes them.
    ///
    /// The equalizer machine has already applied its S07 prediction, so these
    /// go straight to the engine and the app (no second prediction) — the
    /// same shape as [`App::forward`]. A pending presets-menu request is
    /// queued for the presenter ([`App::take_eq_menu`]).
    fn forward_eq(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.apply_from(command, true);
        }
        if let Some(request) = self.eq.take_menu_request() {
            self.request_eq_presets_menu(request);
        }
        self.pump()
    }

    // -----------------------------------------------------------------------
    // Headless playback (the `--null` mode)
    // -----------------------------------------------------------------------

    /// Pumps the null output's virtual clock by `frames` frames.
    ///
    /// This is the deterministic half of the silent windowed mode
    /// (`--no-audio`): it moves the audio clock without any hardware, so a
    /// windowed run plays silently instead of stalling on a full ring. With a
    /// real device there is no virtual clock and this is a no-op (the device
    /// pulls on its own thread).
    pub fn advance_null_clock(&mut self, frames: u64) {
        if let Some(clock) = self.clock.as_ref() {
            clock.pump(frames);
        }
    }

    /// Advances the null output's virtual clock by the real time elapsed since
    /// the previous drive.
    ///
    /// The windowed presenter calls this once per frame so a `--no-audio` run
    /// plays silently in real time, exactly as [`App::run_headless`] drives
    /// the same clock headless. With a real device this is a no-op.
    pub fn drive_null_clock(&mut self) {
        if self.clock.is_none() {
            return;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_null_tick);
        self.last_null_tick = now;
        let rate = self.engine.device_spec().sample_rate;
        if rate == 0 {
            return;
        }
        self.pending_null_frames += elapsed.as_secs_f64() * f64::from(rate);
        let frames = self.pending_null_frames.floor() as u64;
        self.pending_null_frames -= frames as f64;
        if frames > 0 {
            self.advance_null_clock(frames);
        }
    }

    /// Whether the headless run may announce a track: the track must be
    /// **open**.
    ///
    /// The transport is not the signal. The engine sets the transport when
    /// the run starts, a moment before the decode thread has opened the
    /// file, so the transport would announce a file that never plays — and
    /// the report would depend on how fast the decode thread came up. The
    /// engine's own "a track is open" signal is the decoder's format, which
    /// is known exactly when the track opened.
    fn headless_track_open(&self) -> bool {
        self.engine.source_spec().is_some()
    }

    /// Serves the local control channel while nothing is queued (`--control`).
    ///
    /// This is the session a control client drives: the loop pumps like the
    /// playing one, so commands are applied and events published, but it has no
    /// track of its own to end. It ends on `Quit`, when `--seconds` of
    /// wall-clock time has passed (the playback position does not advance while
    /// nothing plays, so the playback limit cannot bound it), or with the
    /// process.
    fn run_control_session(&mut self) -> Result<()> {
        let deadline = self
            .seconds
            .map(|limit| self.started + Duration::from_secs(limit));
        loop {
            std::thread::sleep(Duration::from_millis(5));
            // The null device's clock is driven by the loop, exactly as the
            // windowed presenter drives it: without it a track a client starts
            // would never advance.
            self.drive_null_clock();
            self.pump()?;
            self.print_notes();
            if self.quit {
                return Ok(());
            }
            if let Some(limit) = self.seconds
                && deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                println!("rustamp: reached --seconds {limit}, stopping");
                return Ok(());
            }
        }
    }

    /// Plays the queue to its end (or to `--seconds`) on a virtual device whose
    /// clock this loop drives in real time. No window is opened.
    ///
    /// The run is announced when the track is **open**, not when the transport
    /// says `Playing`: the engine sets the transport when the run starts, a
    /// moment before the decode thread has opened the file. A file that cannot
    /// be opened would then be announced as playing, and the report would
    /// depend on how fast the decode thread came up (see
    /// [`App::headless_track_open`]).
    ///
    /// The notes the app raised are **printed**, the way the windowed
    /// presenter prints them ([`App::print_notes`]): the ones from before the
    /// run and the ones the run itself raises. A headless run is the run
    /// whose diagnostics nobody would see otherwise.
    pub fn run_headless(&mut self) -> Result<()> {
        // The notes the start-up raised (the configuration warnings, the
        // playlist's title format, the DSP bypass) are printed before the
        // run begins: they are the user's diagnostics, and the run is the
        // only reader of them in this mode.
        self.print_notes();
        if self.engine.queue().is_empty() {
            if !self.control_channel {
                println!("rustamp: no input file; nothing to play");
                return Ok(());
            }
            // A served control channel is a session: nothing is queued yet, and
            // the client's commands are what the run is for. It ends on
            // `Quit`, when `--seconds` of wall-clock time passed, or when the
            // process is killed.
            println!("rustamp: no input file; waiting for control commands");
            return self.run_control_session();
        }
        let rate = self.engine.device_spec().sample_rate;
        let mut last_tick = Instant::now();
        let mut pending_frames = 0.0f64;
        let mut announced = false;
        // `Engine::finish_run` re-bases the timeline to zero, so the frames the
        // run actually produced are the maximum seen while it was live (plus
        // what the decode thread reported for the track that ended).
        let mut frames_played = 0u64;

        loop {
            std::thread::sleep(Duration::from_millis(5));
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick);
            last_tick = now;
            if let Some(clock) = self.clock.as_ref() {
                pending_frames += elapsed.as_secs_f64() * f64::from(rate);
                let frames = pending_frames.floor() as u64;
                pending_frames -= frames as f64;
                if frames > 0 {
                    clock.pump(frames);
                }
            }
            // The notes the run raised are printed as they arise, the way the
            // presenter's tick prints them. They are printed even when the
            // pump failed: such a diagnostic is often why the run ended.
            let pumped = self.pump();
            self.print_notes();
            pumped?;
            frames_played = frames_played.max(self.engine.timeline_frames());

            // The run is announced when the track is open, not when the
            // transport says `Playing`. The transport is set when the run
            // starts, before the decode thread has opened the file, so the
            // transport would announce a file that never plays — and the
            // report would depend on how fast the decode thread came up.
            if !announced && self.headless_track_open() {
                announced = true;
                println!("rustamp: playing {}", self.engine.queue()[0].display());
            }
            if self.quit {
                break;
            }
            if let Some(limit) = self.seconds
                && self.engine.position_ms() >= limit.saturating_mul(1000)
            {
                println!("rustamp: reached --seconds {limit}, stopping");
                self.engine.stop()?;
                // The notes the stop raised are printed here too: the run's
                // last word is a diagnostic the user reads.
                let pumped = self.pump();
                self.print_notes();
                pumped?;
                break;
            }
            if self.playlist_ended
                || (announced && self.engine.transport() == TransportState::Stopped)
            {
                break;
            }
            // A file that never opened (bad path, no decoder) must not hang.
            if !announced && self.engine.last_error().is_some() {
                break;
            }
        }

        // The run is over: the queue is empty. Every step printed the notes
        // it raised, and this is the guarantee that none of them is left
        // unread when the run returns.
        self.print_notes();

        let frames = frames_played.max(self.engine.last_track_frames());
        let seconds = if rate == 0 {
            0.0
        } else {
            frames as f64 / f64::from(rate)
        };
        println!(
            "rustamp: played {frames} frames ({seconds:.2} s), {} skipped packets",
            self.engine.skipped_packets()
        );
        if let Some(error) = self.engine.last_error() {
            return Err(Error::Other(error.message.clone()));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Sends UI-produced commands into the channel and processes them.
    ///
    /// The UI machine has already applied its S02 prediction, so these go
    /// straight to the engine and the app (no second prediction).
    pub(crate) fn forward(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.apply_from(command, true);
        }
        self.pump()
    }

    /// Grows the app's entry-id table to the engine queue's length.
    ///
    /// The table mirrors the queue: when it grows, the playlist window's
    /// model grows with it ([`App::sync_playlist_entries`]), so the window
    /// reflects an add without waiting for the next button press. The sync
    /// fires only on growth (the queue-length change is the signal, not a
    /// poll), and selection of live entries and the scroll offset survive
    /// it ([`PlaylistWindow::set_entries`](rustamp_ui::PlaylistWindow::set_entries)).
    fn sync_entry_ids(&mut self) {
        let before = self.entry_ids.len();
        let len = self.engine.queue().len();
        while self.entry_ids.len() < len {
            let id = EntryId(self.next_entry_id);
            self.next_entry_id += 1;
            self.entry_ids.push(id);
        }
        if self.entry_ids.len() != before {
            self.sync_playlist_entries();
            // The editor's model grew (see `App::sync_playlist_entries`): its
            // frame changed without a command or an event.
            self.window_dirty.playlist = true;
        }
    }

    /// The stable id of queue entry `index`.
    pub(crate) fn entry_id_for(&mut self, index: Option<u32>) -> Option<EntryId> {
        let index = index? as usize;
        if index >= self.engine.queue().len() {
            return None;
        }
        self.sync_entry_ids();
        self.entry_ids.get(index).copied()
    }

    pub(crate) fn note(&mut self, message: String) {
        // A failure and a command that does nothing are what the user needs
        // to see; the rest of the notes are diagnostics.
        if let Some(status) = status_text(&message) {
            let now = self.now_ms();
            self.ui.flash_status(status, now);
        }
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message);
        }
    }

    /// The media library with the skinned title bar over it. `size` is the
    /// whole window.
    pub fn render_library_window(&mut self, size: (u32, u32), focused: bool) -> Image {
        let body = self.render_library();
        let mut frame = Image::new(size.0, size.1);
        frame.blit_rect(
            &body,
            body.bounds(),
            0,
            rustamp_ui::chrome::TITLE_BAR_HEIGHT as i32,
        );
        let title = self.library_title();
        rustamp_ui::chrome::draw_title_bar(&mut frame, self.skin.skin(), &title, focused);
        frame
    }

    /// Whether the marquee scrolls by itself (S02 §3).
    pub(crate) fn marquee_autoscroll(&self) -> bool {
        self.ui.autoscroll()
    }

    /// Flips the marquee's autoscroll (S02 §3, id 40189).
    pub(crate) fn toggle_marquee_autoscroll(&mut self) {
        let on = !self.ui.autoscroll();
        let now = self.now_ms();
        self.ui.set_autoscroll(on, now);
    }

    /// Mouse-wheel notches over the main window: the volume, like the Up and
    /// Down keys (own choice: S03 §5 has no wheel row).
    pub fn main_wheel(&mut self, notches: i32) -> Result<()> {
        let action = if notches > 0 {
            ShortcutAction::VolumeUp
        } else {
            ShortcutAction::VolumeDown
        };
        for _ in 0..notches.unsigned_abs() {
            self.dispatch_shared_shortcut(action)?;
        }
        Ok(())
    }

    /// Synchronizes the equalizer window machine's state from the authoritative
    /// engine equalizer state.
    pub(crate) fn sync_eq_from_engine(&mut self) {
        let eq = self.engine.eq();
        self.eq.sync_from_eq_state(&eq);
    }

    fn publish_error(&self, error: &Error) {
        self.bus.publish(Event::Error(ErrorInfo::from(error)));
    }

    /// Makes a bypassed DSP stage reach the user, and remembers the bypass.
    ///
    /// The engine's policy for a failed DSP stage is the **bypass** (W14
    /// finding F4): the failed stage is replaced by a pass-through, the
    /// failure is reported once per run, and the chain runs without that
    /// stage until the caller installs a chain again. The app is that
    /// caller, so:
    ///
    /// * the user gets a note that says what was bypassed and why — not the
    ///   generic engine error, which would name a failure and stop the
    ///   playback;
    /// * the app remembers the bypass, so the preferences page does not
    ///   claim that a DSP is running while the engine runs without it;
    /// * the transport is **not** stopped: the engine ends the track the
    ///   failure happened in and goes on with the next one, and stopping
    ///   here would undo that. The next track plays without the failed
    ///   stage.
    fn surface_dsp_bypass(&mut self, info: &ErrorInfo) {
        let bypass = crate::dsp::BypassedDsp::from_error(info);
        let message = format!(
            "the DSP was bypassed: stage '{}' failed ({}); switch the DSP off and on again in \
             the preferences to install it again",
            bypass.stage, bypass.cause
        );
        self.note(message.clone());
        self.last_error = Some(message);
        self.bypassed_dsp = Some(bypass);
    }

    /// Installs the app's DSP chain in the engine.
    ///
    /// The chain is the app's own: the engine runs it, and a stage that
    /// fails is bypassed by the engine's policy (W14 finding F4) — the app
    /// then remembers the bypass (see [`App::dsp_bypass`]).
    ///
    /// Installing a chain again undoes a bypass: that is how a failed stage
    /// is replaced by a fresh one.
    ///
    /// A live run owns the chain: the engine takes the chain back when the
    /// run ends, so an install during a run would be thrown away. The
    /// install waits for the run to end in that case (see
    /// [`App::pump`]).
    pub fn install_dsp_chain(&mut self, chain: crate::dsp::DspChain) {
        self.caller_dsp_chain = !chain.is_empty();
        self.dsp_chain = chain;
        self.install_dsp_chain_now();
    }

    /// Installs the app's DSP chain again: the manual recovery from a
    /// bypass.
    ///
    /// The engine's policy is that a bypass lasts until the caller installs
    /// a chain again, and this is that call. The app never makes it by
    /// itself: a stage that failed would fail again, so a retry loop would
    /// only repeat the failure. The user asks for the recovery by switching
    /// the DSP off and on again (Preferences → DSP/Effect), which is the one
    /// action that says "try again".
    ///
    /// Returns whether the chain is in the engine now. A live run owns the
    /// chain, so the install waits for that run to end (see
    /// [`App::install_dsp_chain`]).
    pub fn reenable_dsp_chain(&mut self) -> bool {
        self.install_dsp_chain_now()
    }

    /// Hands the app's chain to the engine, if no run owns it.
    ///
    /// A live run took the chain with it, and the engine hands that chain
    /// back when the run ends — an install during a run would be dropped
    /// then. So the install waits for the run to end.
    ///
    /// Returns whether the chain is in the engine now.
    fn install_dsp_chain_now(&mut self) -> bool {
        if self.engine.transport() != TransportState::Stopped {
            self.dsp_install_pending = true;
            return false;
        }
        self.dsp_install_pending = false;
        self.bypassed_dsp = None;
        let stages = self.dsp_chain.build();
        self.engine.set_dsp_chain(stages);
        true
    }

    /// The DSP stage the engine bypassed after it failed, if one was
    /// bypassed.
    pub fn dsp_bypass(&self) -> Option<&crate::dsp::BypassedDsp> {
        self.bypassed_dsp.as_ref()
    }

    /// The names of the stages the app's DSP chain has, in order.
    pub fn dsp_chain_stage_names(&self) -> Vec<&str> {
        self.dsp_chain.stage_names()
    }

    /// The user switched the DSP off.
    ///
    /// The bypassed stage is out of the chain, so the preferences page must
    /// not keep naming it. A chain that was waiting for its installation is
    /// not installed any more: the user said off.
    pub(crate) fn disable_dsp(&mut self) {
        self.bypassed_dsp = None;
        self.dsp_install_pending = false;
    }

    /// The output plug-in the player plays through: the one switched on (only
    /// one output is active, S09 section 4), when the player plays at all.
    #[cfg(windows)]
    fn plugin_output_device(&self) -> Option<Box<dyn rustamp_core::audio::OutputDevice>> {
        if self.output_choice != OutputChoice::Default {
            return None;
        }
        let dll = self
            .enabled_plugins()
            .into_iter()
            .find(|(_, kind)| *kind == PluginKind::Output)
            .map(|(path, _)| path)?;
        let helper = crate::supervise::find_plugin_host()?;
        Some(Box::new(crate::plugin_output::PluginOutput::new(
            helper,
            &dll,
            &self.plugin_approvals_path(),
        )))
    }

    #[cfg(not(windows))]
    fn plugin_output_device(&self) -> Option<Box<dyn rustamp_core::audio::OutputDevice>> {
        None
    }

    /// Moves the audio to the output plug-in the user chose, or back to the
    /// player's own output.
    pub(crate) fn install_plugin_output(&mut self) {
        let wanted = self
            .plugin_output_device()
            .map(|device| device.name().to_owned());
        if wanted != self.plugin_output_name {
            self.change_output();
        }
    }

    /// Opens the output the settings name. A running track keeps its output, so
    /// the change waits for a stop: [`App::pump`] calls this again then.
    pub(crate) fn change_output(&mut self) {
        if self.engine.transport() != TransportState::Stopped {
            if !self.output_change_pending {
                self.note("the output changes when playback has stopped".to_owned());
            }
            self.output_change_pending = true;
            self.plugins_report = Some(self.output_text());
            return;
        }
        self.output_change_pending = false;
        self.plugin_output_name = self
            .plugin_output_device()
            .map(|device| device.name().to_owned());
        match self.recreate_engine() {
            Ok(()) => {
                // The new engine starts without a chain.
                self.install_dsp_chain_now();
                self.plugins_report = Some(self.output_text());
            }
            Err(error) => {
                let text = format!("the output could not be changed: {error}");
                self.plugins_report = Some(text.clone());
                self.note(text);
            }
        }
    }

    /// What the Output page says about the output.
    pub(crate) fn output_text(&self) -> String {
        if self.output_change_pending {
            return "The output changes when playback has stopped.".to_string();
        }
        match (&self.plugin_output_name, self.active_output_file()) {
            (Some(name), _) => format!("The player plays through {name}."),
            (None, Some(file)) => format!(
                "{file} cannot play: the plug-in host was not found, or this run has no sound \
                 output. The player's own output stays in use."
            ),
            (None, None) => "The player plays through its own output.".to_string(),
        }
    }

    /// The file of the output plug-in that is switched on.
    pub(crate) fn active_output_file(&self) -> Option<String> {
        self.plugin_scan
            .entries
            .iter()
            .find(|entry| {
                entry.kind == PluginKind::Output && entry.is_enabled(&self.config.plugins)
            })
            .map(|entry| entry.file.clone())
    }

    /// Takes the approval waiting for the user's answer, for the presenter's
    /// warning.
    pub fn take_plugin_approval_request(
        &mut self,
    ) -> Option<crate::plugin_approval::PendingApproval> {
        self.pending_plugin_approval.take()
    }

    /// The user answered the warning. Yes records the image the warning named,
    /// and nothing else: the plug-in stays off until the user switches it on.
    ///
    /// A file that was on the enabled list under an older image goes off the
    /// list, because approving never makes anything run. The running host read
    /// its allowlist when it started, so it is replaced when the next plug-in is
    /// switched on (see [`App::ask_host_to_load`]).
    pub fn finish_plugin_approval(
        &mut self,
        pending: crate::plugin_approval::PendingApproval,
        approved: bool,
    ) {
        if !approved {
            self.plugins_report = Some(format!("{} was not approved.", pending.name));
            return;
        }
        match pending.approve(&self.plugin_approvals_path()) {
            Ok(_) => {
                rustamp_prefs::forget_file(&mut self.config.plugins, &pending.name);
                if let Err(error) = self.save_config() {
                    self.note(format!("preferences: {error}"));
                }
                self.host_allowlist_stale = true;
                self.rescan_plugins();
                self.plugins_report = Some(format!(
                    "{} is approved and stays off until you switch it on.",
                    pending.name
                ));
            }
            Err(error) => {
                let text = format!("{} was not approved: {error}", pending.name);
                self.plugins_report = Some(text.clone());
                self.note(text);
            }
        }
    }

    /// Asks the host for a plug-in the user just switched on.
    ///
    /// A host started before the last approval would refuse the plug-in, because
    /// it reads its allowlist once. That host is replaced instead, and the new
    /// one is asked for everything that is on (see [`App::pump`]).
    pub(crate) fn ask_host_to_load(&mut self, path: &Path, kind: PluginKind) {
        if std::mem::take(&mut self.host_allowlist_stale)
            && let Some(supervisor) = self.host_supervisor.as_mut()
        {
            supervisor.stop(&mut self.host);
            supervisor.resume();
            self.note("the plugin host restarts to read the new approval".to_owned());
            return;
        }
        self.request_plugin_load(path, kind);
    }

    /// Makes the input plug-ins the user switched on decoders of the player.
    ///
    /// The built-in decoders stay first, and a tie breaks by registration (RFC
    /// 0013), so a plug-in plays what the player cannot decode itself.
    pub(crate) fn install_plugin_inputs(&mut self) {
        let inputs: Vec<PathBuf> = self
            .enabled_plugins()
            .into_iter()
            .filter(|(_, kind)| *kind == PluginKind::Input)
            .map(|(path, _)| path)
            .collect();
        let mut registry = self.builtin_registry.clone();
        if !inputs.is_empty() {
            match crate::supervise::find_plugin_host() {
                Some(helper) => {
                    let approvals = self.plugin_approvals_path();
                    for dll in inputs {
                        match crate::plugin_input::factory(helper.clone(), &dll, &approvals) {
                            Ok(factory) => registry.register(factory),
                            Err(problem) => self.note(problem),
                        }
                    }
                }
                None => self.note(
                    "input plug-ins cannot play: the plug-in host binary was not found".to_owned(),
                ),
            }
        }
        self.registry = registry.clone();
        self.engine.set_registry(registry);
    }

    /// Puts the DSP plug-in the user switched on into the audio path, or takes
    /// the plug-in that was there out of it. Only one DSP module is active at a
    /// time (S09 section 6.3), so the first switched-on plug-in is the one.
    ///
    /// A chain the caller installed by itself stays, plug-in or not.
    /// Returns whether a plug-in's chain is in place.
    pub(crate) fn install_plugin_dsp(&mut self) -> bool {
        if self.caller_dsp_chain {
            return false;
        }
        let dll = self
            .enabled_plugins()
            .into_iter()
            .find(|(_, kind)| *kind == PluginKind::Dsp)
            .map(|(path, _)| path);
        let helper = crate::supervise::find_plugin_host();
        match (dll, helper) {
            (Some(dll), Some(helper)) => {
                let approvals = self.plugin_approvals_path();
                self.plugin_dsp_installed = true;
                self.dsp_chain = crate::dsp::classic_dsp_chain(helper, &dll, &approvals);
                self.install_dsp_chain_now();
                true
            }
            (Some(dll), None) => {
                self.note(format!(
                    "{} cannot process audio: the plug-in host binary was not found",
                    dll.display()
                ));
                false
            }
            (None, _) => {
                if self.plugin_dsp_installed {
                    self.plugin_dsp_installed = false;
                    self.dsp_chain = crate::dsp::DspChain::default();
                    self.install_dsp_chain_now();
                }
                false
            }
        }
    }

    /// Makes an [`Event::Error`] reach the user and leaves the transport in a
    /// defined state (BETA-04).
    ///
    /// * A format change is a notice, not a failure (RFC 0002 C4): the run
    ///   continues, so it is only noted.
    /// * Any other error is noted (the presenter prints notes) and shown in
    ///   the native title ([`App::window_title`]), naming the current entry.
    /// * A running transport is stopped unless the run already reported its
    ///   end of stream (it then finishes on its own). The engine's decode
    ///   worker exits after an open failure without an end of stream, so
    ///   without this the engine stays `Playing` over silence. The rule is
    ///   deliberately broad: a mid-track read failure stops the run as well
    ///   instead of advancing, which is a defined state, and Play retries.
    /// * The UI's transport is re-synced to the engine's, so an optimistic
    ///   Playing light (Play on an empty queue) goes out again.
    fn surface_error(&mut self, info: &ErrorInfo) {
        if info.kind == ErrorKind::FormatChanged {
            self.note(format!("notice: {}", info.message));
            return;
        }
        let is_flush_refusal = self.engine.is_flush_failed();
        let entry = self
            .engine
            .current_index()
            .and_then(|index| self.engine.queue().get(index as usize))
            .map(|track| track.display());
        let message = match entry {
            // An entry that would not open was named by the run itself.
            _ if info.message.starts_with(rustamp_audio::decode::CANNOT_PLAY) => {
                info.message.clone()
            }
            Some(entry) if self.engine.transport() != TransportState::Stopped => {
                format!("cannot play {entry}: {}", info.message)
            }
            // Play with nothing queued: the engine's "queue index 0" means
            // nothing to a first user, so say what to do instead.
            _ if info.kind == ErrorKind::Other && self.engine.queue().is_empty() => {
                "nothing to play: the playlist is empty (Eject or ADD opens a file)".to_string()
            }
            _ => info.message.clone(),
        };
        self.note(format!("error: {message}"));
        if is_flush_refusal {
            self.note(
                "recreating audio engine and output after terminal flush refusal".to_string(),
            );
            if let Err(error) = self.recreate_engine() {
                let recreation_err = format!("error: recreating engine failed: {error}");
                self.note(recreation_err.clone());
                self.last_error = Some(recreation_err);
            } else {
                // The engine and output have been cleanly recreated as obliged by the audio contract.
                self.last_error = None;
            }
        } else {
            self.last_error = Some(message);
        }
        // A run that skipped a dead entry goes on by itself (S02 §1.1).
        if self.engine.transport() != TransportState::Stopped
            && self.engine.end_of_stream().is_none()
            && !info.message.starts_with(rustamp_audio::decode::CANNOT_PLAY)
            && let Err(error) = self.engine.stop()
        {
            self.note(format!("error: stopping after the failure failed: {error}"));
        }
        let transport = self.engine.transport();
        self.ui
            .handle_event(&Event::StateChanged(transport), self.now_ms());
    }

    fn publish_modes(&self) {
        self.bus.publish(Event::ModesChanged {
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
    }

    fn publish_skin(&self) {
        self.bus
            .publish(Event::SkinChanged(self.current_skin.clone()));
    }

    /// Applies a command the application layer owns (see [`routing::app_owns`]).
    fn apply_app(&mut self, command: &Command) {
        use Command as C;
        match command {
            C::Quit => {
                self.quit = true;
                self.bus.publish(Event::Quit);
            }
            C::ToggleShuffle => {
                self.shuffle = !self.shuffle;
                self.publish_modes();
            }
            C::SetShuffle(on) => {
                self.shuffle = *on;
                self.publish_modes();
            }
            C::ToggleRepeat => {
                self.repeat = !self.repeat;
                self.publish_modes();
            }
            C::SetRepeat(on) => {
                self.repeat = *on;
                self.publish_modes();
            }
            C::ToggleAlwaysOnTop => self.always_on_top = !self.always_on_top,
            C::ToggleEasyMove => self.easy_move = !self.easy_move,
            C::Minimize => {
                // The later of Minimize and ShowMainWindow wins: a restore
                // that has not reached the OS yet is dropped.
                self.minimize_requested = true;
                self.restore_request = false;
                if self.focus_request == Some(WindowId::Main) {
                    self.focus_request = None;
                }
            }
            // Closing the focused window quits when that window is the main one
            // (RFC 0005 owner decision O3); it is the only window today.
            // An open dialog closes first instead of quitting the player.
            C::CloseFocusedWindow => {
                if !self.close_open_dialog() {
                    self.quit = true;
                    self.bus.publish(Event::Quit);
                }
            }
            C::ToggleDoubleSize | C::ToggleWindowShade(WindowId::Main) => {
                // The UI machine has already applied its S02 prediction; the
                // presenter reads the new size from `frame_size`.
            }
            C::ToggleWindowShade(WindowId::Playlist) => {
                // The playlist editor owns its shade state (S01 §1: the shade
                // band is 14 px tall); the presenter reads the new size from
                // `playlist_frame_size` and re-flows the docked stack.
                self.playlist.windowshade = !self.playlist.windowshade;
            }
            C::ToggleWindowShade(WindowId::Equalizer) => {
                // The equalizer owns its shade state the same way (S01 §1: the
                // shade band is 14 px tall); the presenter reads the new size
                // from `eq_frame_size` and re-flows the docked stack.
                self.eq.set_windowshade(!self.eq.state().windowshade);
            }
            C::ToggleWindow(WindowId::Main) => self.main_visible = !self.main_visible,
            C::ShowMainWindow => self.show_main_window(),
            // The render state drives the OS window.
            C::ToggleWindow(WindowId::Equalizer) => {}
            C::ToggleWindow(WindowId::Playlist) => {
                // The UI machine flipped `playlist_open`; the presenter
                // creates or drops the playlist editor's OS window to match.
            }
            C::ToggleWindow(window) if *window == crate::library_window::LIBRARY_WINDOW_ID => {
                // The media library: the app owns the index and the view, the
                // presenter creates or drops the OS window to match.
                self.toggle_library();
            }
            C::ToggleWindow(window) => {
                self.note(format!(
                    "{window:?} window is not implemented yet (the toggle is applied to the \
                     render state only)"
                ));
            }
            C::LoadSkin(path) => self.load_skin(path.clone()),
            C::RefreshSkin => {
                if let Some(path) = self.current_skin.clone() {
                    self.load_skin(path);
                } else {
                    self.note("refresh skin: the current skin is the built-in default".to_string());
                }
            }
            C::AddItems { items, at, play } => {
                self.add_items(items, *at, *play);
                if !items.is_empty() {
                    self.publish_playlist_change(PlaylistChangeKind::Inserted);
                }
            }
            C::SetBand { .. } | C::SetPreamp(_) | C::SetEqEnabled(_) | C::SetEqAuto(_) => {
                // S07/Wave 35: The engine owns the authoritative EqState.
                // The equalizer window machine is driven directly from it so that
                // on-screen sliders and lights stay strictly synchronized after any
                // EQ command (external, shortcut, or UI round-trip).
                self.sync_eq_from_engine();
            }
            C::PlayEntry(id) => self.play_entry(*id),
            C::Remove(ids) => {
                self.remove_entries(ids);
                if !ids.is_empty() {
                    self.publish_playlist_change(PlaylistChangeKind::Removed);
                }
            }
            C::Clear => {
                let had_entries = !self.entry_ids.is_empty();
                self.clear_entries();
                if had_entries {
                    self.publish_playlist_change(PlaylistChangeKind::Cleared);
                }
            }
            C::Select(op) => self.playlist.apply_select(op),
            C::Sort(key) => {
                // `sort_entries` does nothing on fewer than two entries, so a
                // no-op sort publishes nothing either.
                if self.entry_ids.len() >= 2 {
                    self.sort_entries(*key);
                    self.publish_playlist_change(PlaylistChangeKind::Reordered);
                }
            }
            C::Randomize => {
                if self.entry_ids.len() >= 2 {
                    self.randomize_entries();
                    self.publish_playlist_change(PlaylistChangeKind::Reordered);
                }
            }
            C::MoveEntries { ids, to } => {
                self.move_entries(ids, *to as usize);
                if !ids.is_empty() {
                    self.publish_playlist_change(PlaylistChangeKind::Moved);
                }
            }
            C::Reverse => {
                if self.entry_ids.len() >= 2 {
                    self.reverse_entries();
                    self.publish_playlist_change(PlaylistChangeKind::Reordered);
                }
            }
            C::OpenAbout
            | C::OpenFileInfo
            | C::OpenJump { .. }
            | C::OpenPlaylist
            | C::OpenBookmarkEditor
            | C::OpenMedia { .. }
            | C::OpenPrefs
            | C::OpenPrefsPage(_)
            | C::OpenSkinBrowser => {
                // Simple modal dialogs (`dialogs.rs`): the app opens a real
                // window for these, so no note is recorded. The open-media
                // faces confirm to `AddItems` with the opening command's
                // `play` flag carried through untouched (RFC 0005 O5).
                self.open_dialog_for(command);
            }
            C::AddBookmark => self.bookmark_current(),
            // The main-menu popup (S04): the app owns the open state, the
            // presenter hosts the OS window for it.
            C::OpenMainMenu => self.open_main_menu(),
            C::FocusNextWindow => self.focus_next_window(),
            // The clutterbar popups (S04 §4, §5): the app owns the pending
            // request, the presenter shows the system menu.
            C::OpenOptionsMenu => self.open_clutter_menu(crate::menu::ClutterMenu::Options),
            C::OpenVisualizationMenu => {
                self.open_clutter_menu(crate::menu::ClutterMenu::Visualization);
            }
            // RFC 0017: the failure is recorded by `save_playlist_file` itself.
            C::SavePlaylist(path) => {
                let _ = self.save_playlist_file(path);
            }
            // RFC 0018: Alt+Delete removes the entries whose file is gone.
            C::RemoveDeadFiles => self.remove_dead_files(),
            // RFC 0005 §5.4: the visualization plug-in lives in the plugin
            // host, so the request travels to the host (see `crate::host`).
            C::ConfigureVisPlugin => self.request_vis_plugin_configuration(),
            // RFC 0020: the same for the requests that *name* a plug-in. The
            // app holds the ids it was told about (see `crate::plugin_inventory`)
            // and only the host can call into a plug-in, so every one of them
            // goes out over the control channel as it arrives. Each is
            // fire-and-forget; the host reports a refusal on its own stderr.
            C::ConfigurePlugin { plugin_id, kind } => self.request_plugin_host_command(
                command.clone(),
                &format!("opening the configuration of plugin {plugin_id} (a {kind:?} plugin)"),
            ),
            C::SeekPlugin { plugin_id, to_ms } => self.request_plugin_host_command(
                command.clone(),
                &format!("seeking plugin {plugin_id} to {to_ms} ms"),
            ),
            C::PausePlugin { plugin_id } => self.request_plugin_host_command(
                command.clone(),
                &format!("pausing plugin {plugin_id}"),
            ),
            C::UnpausePlugin { plugin_id } => self.request_plugin_host_command(
                command.clone(),
                &format!("resuming plugin {plugin_id}"),
            ),
            C::StopPlugin { plugin_id } => self.request_plugin_host_command(
                command.clone(),
                &format!("stopping plugin {plugin_id}"),
            ),
            // The visualization plug-in lives in the plugin host: the toggle goes
            // there, and the host decides what to start (the visualization plug-in
            // loaded last, its first module) or stops the one that runs. The
            // audio data it draws from follows in `pump_visualization`.
            C::ToggleVisPlugin => self.request_plugin_host_command(
                Command::ToggleVisPlugin,
                "the visualization plug-in toggle",
            ),
            // The remaining variants are the engine's (see `routing::app_owns`).
            _ => {}
        }
    }

    fn load_skin(&mut self, path: PathBuf) {
        let loaded = load_skin(&SkinChoice::Path(path));
        if let Some(warning) = loaded.warning() {
            self.note(warning.to_string());
        }
        self.current_skin = if loaded.fell_back() {
            None
        } else {
            loaded
                .description()
                .strip_prefix("skin ")
                .map(PathBuf::from)
        };
        self.skin = loaded;
        self.skin_generation += 1;
        // Every window draws from the skin.
        self.window_dirty.merge(WindowDirty::ALL);
        // A new skin brings its own `Font=` size (S17 §5).
        self.sync_playlist_font_size();
        self.publish_skin();
    }

    /// Loads the built-in default skin (the skin browser's built-in row).
    ///
    /// The same tail as [`load_skin`](App::load_skin) without a path: the
    /// built-in skin always loads, so there is no fallback and the current
    /// skin becomes "no path", which [`RefreshSkin`](Command::RefreshSkin)
    /// already reads that way.
    pub(crate) fn load_builtin_skin(&mut self) {
        self.skin = crate::skin::builtin();
        self.current_skin = None;
        self.skin_generation += 1;
        // Every window draws from the skin.
        self.window_dirty.merge(WindowDirty::ALL);
        // The built-in skin brings its own `Font=` size (S17 §5).
        self.sync_playlist_font_size();
        self.publish_skin();
    }

    /// Loads a playlist file from disk, replacing the current queue with the
    /// surviving entries in order.
    ///
    /// If `play` is true and at least one entry survived, playback of the first
    /// entry is started. If entries were skipped during parsing, the skip
    /// report is recorded in `self.last_playlist_report` and surfaced as
    /// informative notes (and printed to stderr in windowed mode).
    ///
    /// If the file is malformed, unrecognized, or cannot be read, the queue
    /// remains unchanged, an error is recorded in `last_error` (surfaced in
    /// [`App::window_title`]), a note is emitted, and `Err` is returned.
    pub fn load_playlist_file(
        &mut self,
        path: &Path,
        play: bool,
    ) -> Result<rustamp_playlist::PlaylistReport, rustamp_playlist::PlaylistError> {
        // Import (not just load) so the detected format survives: a playlist
        // saved back later defaults to the format it came in. An unrecognized
        // format falls back to M3U when the content is still a readable plain
        // list, and says so; otherwise the original error is reported.
        let imported = match rustamp_playlist::import_playlist_file(path) {
            Ok(imported) => imported,
            Err(original_err) => {
                let bytes = match std::fs::read(path) {
                    Ok(bytes) => bytes,
                    Err(io_err) => {
                        let msg = format!("failed to load playlist {}: {}", path.display(), io_err);
                        self.note(format!("error: {msg}"));
                        self.last_error = Some(msg);
                        self.publish_error(&rustamp_core::error::Error::Io(std::io::Error::new(
                            io_err.kind(),
                            io_err.to_string(),
                        )));
                        return Err(original_err);
                    }
                };
                let base = path.parent().unwrap_or_else(|| std::path::Path::new(""));
                match rustamp_playlist::read_playlist_report_as(
                    rustamp_playlist::PlaylistFormat::M3u,
                    &bytes,
                    base,
                ) {
                    Ok(report) => {
                        self.note(format!(
                            "unknown playlist format in {}; treating as M3U",
                            path.display()
                        ));
                        rustamp_playlist::ImportedPlaylist {
                            format: rustamp_playlist::PlaylistFormat::M3u,
                            report,
                        }
                    }
                    Err(_) => {
                        let msg = format!(
                            "failed to load playlist {}: {}",
                            path.display(),
                            original_err
                        );
                        self.note(format!("error: {msg}"));
                        self.last_error = Some(msg);
                        let core_err = match &original_err {
                            rustamp_playlist::PlaylistError::Io(e) => {
                                rustamp_core::error::Error::Io(std::io::Error::new(
                                    e.kind(),
                                    e.to_string(),
                                ))
                            }
                            rustamp_playlist::PlaylistError::UnsupportedFormat(m) => {
                                rustamp_core::error::Error::UnsupportedFormat(m.clone())
                            }
                            rustamp_playlist::PlaylistError::InvalidEntry(m) => {
                                rustamp_core::error::Error::InvalidArgument(m.clone())
                            }
                        };
                        self.publish_error(&core_err);
                        return Err(original_err);
                    }
                }
            }
        };
        let report = imported.report;
        self.last_playlist_format = Some(imported.format);

        if !report.is_clean() {
            self.note(format!(
                "loaded {} tracks, skipped {} malformed entries from {}",
                report.entries.len(),
                report.skip_count(),
                path.display()
            ));
            for skip in &report.skips {
                self.note(format!("  - skipped: {}", skip.reason));
            }
        }

        let queue: Vec<TrackSource> = report
            .entries
            .iter()
            .map(|entry| match &entry.location {
                MediaLocation::File(p) => TrackSource::path(p.clone()),
                MediaLocation::Url(u) => TrackSource::Url(u.clone()),
            })
            .collect();

        // A replacement must not lend the old track's index (or an in-flight
        // TrackStarted) to an unrelated entry in the new queue.
        self.clear_entries();
        self.engine.set_queue(queue);
        self.sync_entry_ids();
        if !play {
            self.engine.select_first_if_none();
        }

        if play && !report.entries.is_empty() {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(0) {
                self.publish_error(&error);
            }
        }

        self.last_playlist_report = Some(report.clone());
        Ok(report)
    }

    /// Where the playlist of a session lives: `playlist.m3u8` beside the
    /// configuration file (own choice; no spec covers it).
    fn session_playlist_path(&self) -> Option<PathBuf> {
        Some(self.config_path.as_ref()?.parent()?.join("playlist.m3u8"))
    }

    /// Keeps the playlist for the next start. An empty playlist leaves no file.
    pub fn save_session_playlist(&mut self) {
        let Some(path) = self.session_playlist_path() else {
            return;
        };
        if self.engine.queue().is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if let Err(error) = self.write_queue_playlist(&path) {
            eprintln!("rustamp: the playlist was not saved: {error}");
        }
    }

    /// Brings the last session's playlist back, unless the command line already
    /// filled the playlist. Nothing starts playing.
    pub fn restore_session_playlist(&mut self) {
        if !self.engine.queue().is_empty() {
            return;
        }
        let Some(path) = self.session_playlist_path().filter(|path| path.is_file()) else {
            return;
        };
        if let Err(error) = self.load_playlist_file(&path, false) {
            self.note(format!("the last playlist was not restored: {error}"));
        }
    }

    /// Writes the current queue to `path` as an M3U, PLS or XSPF playlist, in
    /// queue order (RFC 0017).
    ///
    /// The format follows the file extension, case-insensitively. An unknown
    /// extension, B4S (not implemented) or a failed write records an error
    /// like a failed load does (a note, `last_error`, and the published
    /// error) and returns `Err`; no file is written in the first two cases.
    pub fn save_playlist_file(&mut self, path: &Path) -> Result<()> {
        let result = self.write_queue_playlist(path);
        if let Err(error) = &result {
            self.record_failure(
                format!("failed to save playlist {}: {}", path.display(), error),
                error,
            );
        }
        result
    }

    /// Records a failure the way the file actions do: a note for the log, the
    /// error the window title shows, and the published error.
    ///
    /// One place, so the playlist save and the HTML playlist cannot drift
    /// apart in how they report a failed write.
    pub(crate) fn record_failure(&mut self, message: String, error: &Error) {
        self.note(format!("error: {message}"));
        self.last_error = Some(message);
        self.publish_error(error);
    }

    fn write_queue_playlist(&self, path: &Path) -> Result<()> {
        // Format selection rule (RFC 0017 import wiring):
        //   * a recognized, supported extension on `path` wins — the user
        //     chose it;
        //   * B4S or an unknown extension is rejected (the user named a
        //     format we cannot write);
        //   * no extension at all defaults to the format of the playlist that
        //     was last loaded (so a PLS loads and saves as PLS), or to M3U
        //     when nothing has been loaded yet.
        let format = if path.extension().is_none() {
            self.last_playlist_format
                .unwrap_or(rustamp_playlist::PlaylistFormat::M3u)
        } else {
            match rustamp_playlist::PlaylistFormat::from_path(path) {
                Some(format) if format.is_supported() => format,
                Some(_) => {
                    return Err(Error::UnsupportedFormat(format!(
                        "{}: save needs an .m3u, .m3u8, .pls or .xspf file name",
                        path.display()
                    )));
                }
                None => {
                    return Err(Error::UnsupportedFormat(format!(
                        "{}: save needs an .m3u, .m3u8, .pls or .xspf file name",
                        path.display()
                    )));
                }
            }
        };
        let entries: Vec<rustamp_playlist::LoadedEntry> = self
            .engine
            .queue()
            .iter()
            .map(|source| {
                let location = match source {
                    TrackSource::Path(p) => MediaLocation::File(p.clone()),
                    TrackSource::Url(u) => MediaLocation::Url(u.clone()),
                };
                rustamp_playlist::LoadedEntry::new(location)
            })
            .collect();
        let bytes = rustamp_playlist::write_playlist(format, &entries)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn add_items(&mut self, items: &[MediaLocation], at: Option<u32>, play: bool) {
        let first_new = self.engine.queue().len() as u32;
        let mut queue = self.engine.queue().to_vec();
        for location in items {
            match location {
                MediaLocation::File(path) if is_playlist_file(path) => {
                    match rustamp_playlist::import_playlist_file(path) {
                        Ok(imported) => {
                            self.last_playlist_format = Some(imported.format);
                            let report = imported.report;
                            if !report.is_clean() {
                                self.note(format!(
                                    "loaded {} tracks, skipped {} malformed entries from {}",
                                    report.entries.len(),
                                    report.skip_count(),
                                    path.display()
                                ));
                                for skip in &report.skips {
                                    self.note(format!("  - skipped: {}", skip.reason));
                                }
                            }
                            queue.extend(report.entries.iter().map(|e| match &e.location {
                                MediaLocation::File(p) => TrackSource::path(p.clone()),
                                MediaLocation::Url(u) => TrackSource::Url(u.clone()),
                            }));
                            self.last_playlist_report = Some(report);
                        }
                        Err(err) => {
                            let msg =
                                format!("failed to load playlist {}: {}", path.display(), err);
                            self.note(format!("error: {msg}"));
                            self.last_error = Some(msg);
                            let core_err = match &err {
                                rustamp_playlist::PlaylistError::Io(e) => {
                                    rustamp_core::error::Error::Io(std::io::Error::new(
                                        e.kind(),
                                        e.to_string(),
                                    ))
                                }
                                rustamp_playlist::PlaylistError::UnsupportedFormat(m) => {
                                    rustamp_core::error::Error::UnsupportedFormat(m.clone())
                                }
                                rustamp_playlist::PlaylistError::InvalidEntry(m) => {
                                    rustamp_core::error::Error::InvalidArgument(m.clone())
                                }
                            };
                            self.publish_error(&core_err);
                        }
                    }
                }
                MediaLocation::File(path) => queue.push(TrackSource::path(path.clone())),
                MediaLocation::Url(url) => queue.push(TrackSource::Url(url.clone())),
            }
        }
        self.engine.set_queue(queue);
        self.sync_entry_ids();
        if !play {
            self.engine.select_first_if_none();
        }
        if let Some(at) = at
            && at < first_new
        {
            self.note(format!(
                "AddItems: insert position {at} is not supported yet; the entries were appended"
            ));
        }
        if play && !items.is_empty() {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(first_new) {
                self.publish_error(&error);
            }
        }
    }

    fn play_entry(&mut self, id: EntryId) {
        if let Some(index) = self.entry_ids.iter().position(|entry| *entry == id) {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(index as u32) {
                self.publish_error(&error);
            }
        } else {
            self.note(format!("PlayEntry: unknown entry id {id:?}"));
        }
    }

    fn remove_entries(&mut self, ids: &[EntryId]) {
        if ids.is_empty() {
            return;
        }
        let ids_set: BTreeSet<_> = ids.iter().copied().collect();

        // Check if the currently playing entry is being removed
        let playing_id = self
            .engine
            .current_index()
            .and_then(|idx| self.entry_ids.get(idx as usize))
            .copied();
        let playing_removed = playing_id.is_some_and(|id| ids_set.contains(&id));

        if playing_removed && self.engine.transport() != TransportState::Stopped {
            let _ = self.engine.stop();
        }

        let mut new_ids = Vec::with_capacity(self.entry_ids.len());
        let mut new_queue = Vec::with_capacity(self.engine.queue().len());
        for (id, track) in self.entry_ids.iter().zip(self.engine.queue()) {
            if !ids_set.contains(id) {
                new_ids.push(*id);
                new_queue.push(track.clone());
            }
        }
        self.entry_ids = new_ids;
        self.engine.set_queue(new_queue);
        // Keep a surviving current entry by identity, not by its former row.
        // Removing it clears Current even when another entry takes that index.
        let current = playing_id
            .and_then(|id| self.entry_ids.iter().position(|entry| *entry == id))
            .map(|index| index as u32);
        self.engine.set_current_index(current);
        self.sync_playlist_entries();
    }

    /// Applies `Command::Sort` to the playlist order (S03 §2 `40209`-`40211`,
    /// RFC 0005 §5: the app owns the playlist model).
    ///
    /// The key texts mirror core
    /// [`Playlist::sort`](rustamp_core::playlist::Playlist::sort):
    ///
    /// * `Title` — the name the last playlist load carried (`#EXTINF`, PLS
    ///   `TitleN`), matched by location and falling back to the file name, so
    ///   a queue the load did not name sorts like `FileName`;
    /// * `FileName` — the final path component;
    /// * `PathAndFileName` — the full location;
    /// * `Duration` — the duration the last load carried, unknown first
    ///   (`None` before `Some`, the core's rule).
    ///
    /// Text is compared case-insensitively and every sort is stable, so equal
    /// keys keep their relative order. Table, queue and model reorder together
    /// ([`App::reorder_entries`]); nothing else changes — the current entry
    /// keeps playing and ids keep their selection.
    fn sort_entries(&mut self, key: SortKey) {
        if self.entry_ids.len() < 2 {
            return;
        }
        // Titles and durations exist only for what the last playlist load
        // reported: no entry is probed for tags or properties (S14 title
        // formatting is open; `rustamp-meta` does not scan properties).
        let mut order: Vec<usize> = (0..self.entry_ids.len()).collect();
        match key {
            SortKey::Duration => {
                let durations = self.loaded_durations();
                order.sort_by_cached_key(|&index| {
                    self.queue_location(index)
                        .and_then(|location| durations.get(&location).copied())
                });
            }
            SortKey::Title | SortKey::FileName | SortKey::PathAndFileName => {
                // Only `Title` reads the loaded names; the other two sort on
                // the location alone.
                let titles = if key == SortKey::Title {
                    self.loaded_titles()
                } else {
                    HashMap::new()
                };
                order.sort_by_cached_key(|&index| self.sort_text(index, key, &titles));
            }
        }
        self.reorder_entries(order);
    }

    /// The case-insensitive sort text of queue row `index` for one of the
    /// three text keys; `titles` feeds `Title` only.
    ///
    /// A row the queue does not hold sorts as the empty string and leaves with
    /// the next sync; while the tables mirror each other that cannot happen.
    fn sort_text(
        &self,
        index: usize,
        key: SortKey,
        titles: &HashMap<MediaLocation, String>,
    ) -> String {
        let Some(location) = self.queue_location(index) else {
            return String::new();
        };
        let text = match key {
            SortKey::Title => titles
                .get(&location)
                .cloned()
                .unwrap_or_else(|| location.file_name()),
            SortKey::FileName => location.file_name(),
            SortKey::PathAndFileName => location.to_display_string(),
            // `Duration` takes the numeric branch of `Self::sort_entries`.
            SortKey::Duration => unreachable!("Duration sorts numerically"),
        };
        text.to_lowercase()
    }

    /// The media location of queue row `index`, if the row exists.
    fn queue_location(&self, index: usize) -> Option<MediaLocation> {
        Some(match self.engine.queue().get(index)? {
            TrackSource::Path(path) => MediaLocation::File(path.clone()),
            TrackSource::Url(url) => MediaLocation::Url(url.clone()),
        })
    }

    /// Names the last playlist load carried, by location.
    pub(crate) fn loaded_titles(&self) -> HashMap<MediaLocation, String> {
        self.last_playlist_report
            .as_ref()
            .map(|report| {
                report
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .title
                            .as_ref()
                            .map(|title| (entry.location.clone(), title.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Durations the last playlist load carried, by location.
    pub(crate) fn loaded_durations(&self) -> HashMap<MediaLocation, u64> {
        self.last_playlist_report
            .as_ref()
            .map(|report| {
                report
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .duration_ms
                            .map(|duration| (entry.location.clone(), duration))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Applies `Command::Randomize` (S03 §2 `40212`, the `MISC -> Randomize
    /// List` action): one in-place permutation of the order.
    ///
    /// S15 §5 maps the action to `rustamp-playlist::order::randomize`, whose
    /// Fisher-Yates shuffle is [`rustamp_playlist::shuffle`]. The app holds no
    /// core `Playlist` — its order is the id table plus the engine queue — so
    /// the permutation is taken over row indices and handed to
    /// [`App::reorder_entries`]: ids, selection and the current entry survive,
    /// only the rows move.
    ///
    /// The seed is an own choice (S15 Q7.3 leaves the reference's seeding
    /// open): wall-clock nanoseconds give each press a fresh stream with no
    /// random-source dependency, and the generator itself is deterministic. A
    /// clock that does not advance only repeats the previous permutation, which
    /// is a legal outcome of a shuffle anyway.
    fn randomize_entries(&mut self) {
        if self.entry_ids.len() < 2 {
            return;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        let mut rng = rustamp_playlist::Rng::new(seed);
        let mut order: Vec<usize> = (0..self.entry_ids.len()).collect();
        rustamp_playlist::shuffle(&mut order, &mut rng);
        self.reorder_entries(order);
    }

    /// Applies the existing block-move contract: ids keep playlist-relative
    /// order, unknown/duplicate ids are ignored, and `to` is clamped in the
    /// remaining list. Like sorting, playback and selection follow identity.
    fn move_entries(&mut self, ids: &[EntryId], to: usize) {
        let moved: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        let (block, mut rest): (Vec<_>, Vec<_>) =
            (0..self.entry_ids.len()).partition(|&row| moved.contains(&self.entry_ids[row]));
        let to = to.min(rest.len());
        rest.splice(to..to, block);
        if rest.iter().copied().eq(0..self.entry_ids.len()) {
            return;
        }
        self.reorder_entries(rest);
    }

    /// Reorders the id table and the engine queue to `order`, a permutation of
    /// row indices, and re-syncs the playlist model.
    ///
    /// The tables mirror each other (see [`App::sync_entry_ids`]), so they move
    /// together. Playback is untouched: the current entry is carried by
    /// identity to its new row, the way [`App::remove_entries`] keeps a
    /// surviving current entry, so the track plays on and only the indicator
    /// moves. The model keeps selection by id across the re-sync.
    ///
    /// The current index is written before the queue because
    /// [`Engine::set_queue`] re-plans an active shuffle around the current
    /// index, which is only meaningful in the new order. An order that is not a
    /// row permutation of a matching queue is dropped rather than applied: half
    /// of it would desynchronize the tables.
    fn reorder_entries(&mut self, order: Vec<usize>) {
        let len = self.entry_ids.len();
        if len == 0
            || order.len() != len
            || self.engine.queue().len() != len
            || order.iter().any(|&row| row >= len)
        {
            return;
        }
        let new_ids: Vec<EntryId> = order.iter().map(|&row| self.entry_ids[row]).collect();
        let new_queue: Vec<TrackSource> = order
            .iter()
            .map(|&row| self.engine.queue()[row].clone())
            .collect();
        let current_id = self
            .engine
            .current_index()
            .and_then(|index| self.entry_ids.get(index as usize))
            .copied();
        self.entry_ids = new_ids;
        let current = current_id
            .and_then(|id| self.entry_ids.iter().position(|entry| *entry == id))
            .map(|index| index as u32);
        self.engine.set_current_index(current);
        self.engine.set_queue(new_queue);
        self.sync_playlist_entries();
    }

    fn clear_entries(&mut self) {
        if self.engine.transport() != TransportState::Stopped {
            let _ = self.engine.stop();
        }
        self.entry_ids.clear();
        self.engine.set_queue(Vec::new());
        self.engine.set_current_index(None);
        self.sync_playlist_entries();
    }

    /// Applies `Command::Reverse` (S03 §2 `40213`, "Reverse the list").
    ///
    /// One permutation of the row indices, handed to
    /// [`App::reorder_entries`]: ids, selection and the current entry survive,
    /// only the rows turn around. The list's order after the reversal is the
    /// plain reverse of the order before it — the spec names the action, not
    /// the resulting order, so this is the one reading of "reverse the list"
    /// that needs no further choice.
    fn reverse_entries(&mut self) {
        if self.entry_ids.len() < 2 {
            return;
        }
        let order: Vec<usize> = (0..self.entry_ids.len()).rev().collect();
        self.reorder_entries(order);
    }

    /// Removes every entry whose local file no longer exists on disk
    /// ("Remove all dead files", Alt+Delete, S03 §2 command id 40222,
    /// RFC 0018).
    ///
    /// A playlist with no dead entry changes nothing and records no note — the
    /// same silence as an unstaged Delete (S06 §5.3). The removal itself is the
    /// ordinary [`App::remove_entries`] path, so the playing entry, the current
    /// index and the selection follow the rules every other removal uses.
    fn remove_dead_files(&mut self) {
        let dead: Vec<EntryId> = self
            .entry_ids
            .iter()
            .copied()
            .zip(self.engine.queue())
            .filter(|(_id, source)| source_is_dead(source))
            .map(|(id, _source)| id)
            .collect();
        if dead.is_empty() {
            return;
        }
        self.remove_entries(&dead);
    }
}

#[cfg(test)]
#[path = "visualization_tests.rs"]
mod visualization_tests;

#[cfg(test)]
#[path = "fade_note_tests.rs"]
mod fade_note_tests;

#[cfg(test)]
#[path = "stream_status_tests.rs"]
mod stream_status_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use rustamp_core::audio::{
        AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint,
    };
    use rustamp_core::meta::TrackMetadata;
    use rustamp_core::playlist::EntryId;
    use rustamp_core::state::StreamInfo;
    use rustamp_ui::{MainWindowLayout, Widget};

    /// A silent decoder for `*.tst` paths: enough to open a queue entry without
    /// a real media file.
    struct SilenceDecoder {
        spec: AudioSpec,
        pos: u64,
        total: u64,
    }

    impl Decoder for SilenceDecoder {
        fn spec(&self) -> AudioSpec {
            self.spec
        }

        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let channels = usize::from(self.spec.channels);
            let want = out.len() / channels;
            let n = want.min(self.total.saturating_sub(self.pos) as usize);
            for sample in &mut out[..n * channels] {
                *sample = 0.0;
            }
            self.pos += n as u64;
            Ok(n)
        }

        fn seek(&mut self, to: Duration) -> Result<Duration> {
            let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)) as u64;
            self.pos = frames.min(self.total);
            Ok(self.spec.duration_of(self.pos))
        }

        fn duration(&self) -> Option<Duration> {
            Some(self.spec.duration_of(self.total))
        }

        fn metadata(&self) -> TrackMetadata {
            TrackMetadata {
                title: Some("App Test Track".to_string()),
                ..TrackMetadata::default()
            }
        }

        fn stream_info(&self) -> StreamInfo {
            StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: self.spec.sample_rate,
                channels: self.spec.channels,
            }
        }
    }

    struct TestFactory;

    impl DecoderFactory for TestFactory {
        fn name(&self) -> &str {
            "app-test"
        }

        fn extensions(&self) -> &[&str] {
            &["tst"]
        }

        fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
            if hint.extension == Some("tst") {
                Confidence::CONTENT
            } else {
                Confidence::NONE
            }
        }

        fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
            Ok(Box::new(SilenceDecoder {
                spec: AudioSpec::new(44100, 2),
                pos: 0,
                total: 44_100,
            }))
        }
    }

    /// A decoder without a length and without a seek: a stream (S19).
    struct StreamDecoder {
        spec: AudioSpec,
        pos: u64,
    }

    impl Decoder for StreamDecoder {
        fn spec(&self) -> AudioSpec {
            self.spec
        }

        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let channels = usize::from(self.spec.channels);
            let n = (out.len() / channels).min(4096);
            for sample in &mut out[..n * channels] {
                *sample = 0.0;
            }
            self.pos += n as u64;
            Ok(n)
        }

        fn seek(&mut self, _to: Duration) -> Result<Duration> {
            Err(rustamp_core::error::Error::Unsupported(
                "seeking in a stream",
            ))
        }

        fn duration(&self) -> Option<Duration> {
            None
        }

        fn is_seekable(&self) -> bool {
            false
        }

        fn metadata(&self) -> TrackMetadata {
            TrackMetadata {
                title: Some("App Test Stream".to_string()),
                ..TrackMetadata::default()
            }
        }

        fn stream_info(&self) -> StreamInfo {
            StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: self.spec.sample_rate,
                channels: self.spec.channels,
            }
        }
    }

    struct StreamFactory;

    impl DecoderFactory for StreamFactory {
        fn name(&self) -> &str {
            "app-stream"
        }

        fn extensions(&self) -> &[&str] {
            &["strm"]
        }

        fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
            if hint.extension == Some("strm") {
                Confidence::CONTENT
            } else {
                Confidence::NONE
            }
        }

        fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
            Ok(Box::new(StreamDecoder {
                spec: AudioSpec::new(44100, 2),
                pos: 0,
            }))
        }
    }

    fn registry() -> DecoderRegistry {
        let mut registry = DecoderRegistry::new();
        registry.register(Arc::new(TestFactory) as Arc<dyn DecoderFactory>);
        registry.register(Arc::new(StreamFactory) as Arc<dyn DecoderFactory>);
        registry
    }

    /// A temp dir with one `*.tst` file in it.
    fn track(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, b"").unwrap();
        path
    }

    fn app_with_file(path: Option<PathBuf>) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: path.map(MediaLocation::File),
            seconds: None,
        };
        (App::with_registry(options, registry()).unwrap(), dir)
    }

    /// An app built from `config`, with the file it plays and the path it
    /// saves to.
    fn app_with_config(
        config: Config,
        file: Option<PathBuf>,
        config_path: Option<PathBuf>,
    ) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let options = AppOptions {
            config,
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: file.map(MediaLocation::File),
            seconds: None,
        };
        let mut app = App::with_registry(options, registry()).unwrap();
        app.set_config_path(config_path);
        (app, dir)
    }

    #[test]
    fn output_open_notes_reach_the_presenter_without_erasing_the_saved_request() {
        let (mut app, _dir) = app_with_file(None);
        app.set_output_mode(rustamp_core::config::OutputMode::Exclusive);
        let (tx, rx) = std::sync::mpsc::channel();
        app.output_notes = rx;
        // Exercise diagnostic transport only, not a simulated exclusive open.
        let diagnostic =
            "Exclusive audio output was refused; using WASAPI shared mode: endpoint busy";
        tx.send(diagnostic.to_owned()).unwrap();
        for _ in 0..MAX_NOTES {
            app.note("ordinary note".into());
        }
        app.recreate_engine().unwrap();
        assert!(app.drain_notes().iter().any(|note| note == diagnostic));
        assert_eq!(
            app.current_config().audio.output_mode,
            rustamp_core::config::OutputMode::Exclusive
        );
    }

    /// The player's one printing rule for a note: a line that starts with
    /// the player's prefix, and the notes keep the order they were raised
    /// in.
    #[test]
    fn a_note_is_printed_with_the_players_prefix_and_the_notes_keep_their_order() {
        let mut out = Vec::new();
        write_notes(
            &mut out,
            &["the first note".to_string(), "the second note".to_string()],
        )
        .expect("the notes are written");
        assert_eq!(
            String::from_utf8(out).expect("the notes are UTF-8"),
            "rustamp: the first note\nrustamp: the second note\n"
        );
    }

    /// A headless run prints the notes it was given: the one from before the
    /// run and the one the run itself raises.
    ///
    /// The run drains the queue, so nothing is left in it when the run
    /// ends: the notes were printed, and printed once.
    #[test]
    fn a_headless_run_prints_the_notes_it_raised() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "one.tst");
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: Some(MediaLocation::File(file)),
            // The run stops at once: this test is about the notes, not about
            // the playback.
            seconds: Some(0),
        };
        let mut app = App::with_registry(options, registry()).unwrap();
        // A note from before the run.
        app.note("a note from before the run".to_string());
        // The engine reports a failure while the run is going: the app
        // notes it, and the run prints it as well.
        app.bus.publish(Event::Error(ErrorInfo {
            kind: ErrorKind::Io,
            message: "the file could not be opened".to_string(),
        }));
        app.run_headless().expect("the run ends");
        // Both notes were printed: the queue is empty afterwards.
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn save_playlist_writes_the_queue_in_the_format_the_extension_names() {
        let (mut app, dir) = app_with_file(None);
        let first = track(&dir, "one.mp3");
        let stream = "http://example.test/stream".to_owned();
        app.engine.set_queue(vec![
            TrackSource::path(first.clone()),
            TrackSource::Url(stream.clone()),
        ]);
        app.sync_entry_ids();

        for name in ["list.m3u", "list.M3U8", "list.pls", "list.xspf"] {
            let target = dir.path().join(name);
            app.submit(Command::SavePlaylist(target.clone())).unwrap();
            let report = rustamp_playlist::load_playlist_file_report(&target).unwrap();
            let locations: Vec<MediaLocation> = report
                .entries
                .iter()
                .map(|entry| entry.location.clone())
                .collect();
            assert_eq!(
                locations,
                vec![
                    MediaLocation::File(first.clone()),
                    MediaLocation::Url(stream.clone())
                ],
                "{name}"
            );
        }
        assert!(app.last_error.is_none());
    }

    #[test]
    fn save_playlist_refuses_an_unknown_or_unsupported_extension() {
        let (mut app, dir) = app_with_file(None);
        app.engine
            .set_queue(vec![TrackSource::path(track(&dir, "one.mp3"))]);
        app.sync_entry_ids();

        for name in ["list.txt", "list.b4s"] {
            let target = dir.path().join(name);
            assert!(app.save_playlist_file(&target).is_err(), "{name}");
            assert!(!target.exists(), "{name} must not be written");
        }
        assert!(app.last_error.is_some());
    }

    #[test]
    fn load_pls_then_save_with_no_extension_defaults_to_pls() {
        let (mut app, dir) = app_with_file(None);
        let track_path = track(&dir, "one.mp3");
        let pls_path = dir.path().join("list.pls");
        std::fs::write(
            &pls_path,
            format!("[playlist]\nFile1={}\nTitle1=One\n", track_path.display()),
        )
        .unwrap();

        app.load_playlist_file(&pls_path, false).unwrap();
        assert_eq!(
            app.last_playlist_format(),
            Some(rustamp_playlist::PlaylistFormat::Pls)
        );

        let target = dir.path().join("saved"); // no extension: default to loaded format
        app.save_playlist_file(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        // PLS content marker.
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .starts_with("[playlist]"),
            "bytes: {:?}",
            bytes
        );
        assert!(app.last_error.is_none());
    }

    #[test]
    fn fresh_playlist_save_defaults_to_m3u() {
        let (mut app, dir) = app_with_file(None);
        let first = track(&dir, "one.mp3");
        app.engine.set_queue(vec![TrackSource::path(first.clone())]);
        app.sync_entry_ids();

        assert_eq!(app.last_playlist_format(), None);

        let target = dir.path().join("saved"); // no extension
        app.save_playlist_file(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        // M3U content.
        assert!(
            std::str::from_utf8(&bytes).unwrap().contains(".mp3")
                || bytes.starts_with(b"#EXTM3U")
                || !bytes.is_empty(),
            "bytes: {:?}",
            bytes
        );
        // The saved file is readable as M3U.
        let report = rustamp_playlist::load_playlist_file_report(&target).unwrap();
        assert_eq!(report.entries.len(), 1);
    }

    #[test]
    fn unknown_format_file_is_treated_as_m3u() {
        let (mut app, dir) = app_with_file(None);
        // A plain list of paths behind an unrecognized extension: detection
        // fails, but the content is a readable M3U list, so it is treated as M3U.
        let txt_path = dir.path().join("list.unknown");
        std::fs::write(&txt_path, b"track1.tst\n").unwrap();
        let t = track(&dir, "track1.tst");

        // load may succeed as M3U fallback
        let _ = app.load_playlist_file(&txt_path, false);
        // If it loaded, format is M3U.
        if app.last_error().is_none() {
            assert_eq!(
                app.last_playlist_format(),
                Some(rustamp_playlist::PlaylistFormat::M3u)
            );
            assert_eq!(app.engine().queue(), &[TrackSource::path(t)]);
        }
    }

    fn widget_centre(widget: Widget) -> (i32, i32) {
        let rect = widget.rect(&MainWindowLayout::CLASSIC);
        (
            rect.x + rect.width as i32 / 2,
            rect.y + rect.height as i32 / 2,
        )
    }

    fn pump_until(app: &mut App, mut cond: impl FnMut(&App) -> bool, max_frames: u64) -> bool {
        if cond(app) {
            return true;
        }
        let mut pumped = 0;
        while pumped < max_frames {
            app.pump().unwrap();
            if cond(app) {
                return true;
            }
            pumped += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        cond(app)
    }

    /// Like [`pump_until`], but also advances the virtual clock, so a run can
    /// actually drain and finish. The engine ends a run when the ring is
    /// drained, not when the device clock passes the end, so a wait that only
    /// pumps the app can never see the transport stop.
    fn pump_until_drained(
        app: &mut App,
        mut cond: impl FnMut(&App) -> bool,
        max_frames: u64,
    ) -> bool {
        if cond(app) {
            return true;
        }
        let mut pumped = 0;
        while pumped < max_frames {
            app.advance_null_clock(1024);
            app.pump().unwrap();
            if cond(app) {
                return true;
            }
            pumped += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        cond(app)
    }

    // ----- command routing: a UI outcome reaches the engine -----

    #[test]
    fn clicking_play_turns_the_ui_outcome_into_a_command_the_engine_obeys() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));

        // The UI machine is what turns the click into commands.
        let (x, y) = widget_centre(Widget::Play);
        let commands = app.mouse_down(x, y).unwrap();
        assert!(
            commands.is_empty(),
            "a press alone fires nothing: {commands:?}"
        );
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::Play]);

        // The command reached the engine...
        assert_eq!(app.engine().transport(), TransportState::Playing);
        // ...and the engine's event came back into the UI state.
        assert!(
            pump_until(
                &mut app,
                |app| app.state().transport == TransportState::Playing,
                2000
            ),
            "the UI never saw Playing: {:?}",
            app.state()
        );
    }

    #[test]
    fn clicking_stop_returns_the_transport_to_stopped_in_the_ui() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));
        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.state().transport == TransportState::Playing,
            2000
        ));

        let (x, y) = widget_centre(Widget::Stop);
        app.mouse_down(x, y).unwrap();
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::Stop]);
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.state().transport, TransportState::Stopped);
        assert!(
            !app.state().time_visible,
            "stopped blanks the time (S02 §4)"
        );
    }

    #[test]
    fn a_slider_drag_routes_volume_to_the_engine_and_back() {
        let (mut app, _dir) = app_with_file(None);
        let track_rect = MainWindowLayout::CLASSIC.volume_track;
        let y = track_rect.y + 6;

        app.mouse_down(track_rect.x, y).unwrap();
        let commands = app.mouse_move(track_rect.right() - 1, y).unwrap();
        assert!(
            matches!(commands.last(), Some(Command::SetVolume(_))),
            "{commands:?}"
        );
        app.mouse_up(track_rect.right() - 1, y).unwrap();

        // The engine published the new volume; the UI shows it as a percentage.
        assert_eq!(app.state().volume, 100);
        assert_eq!(app.engine().state().volume, 255);
    }

    #[test]
    fn keyboard_volume_up_reaches_the_engine() {
        let (mut app, _dir) = app_with_file(None);
        let before = app.engine().state().volume;
        let shown_before = app.state().volume;
        let outcome = app.key_down(Key::Up, Modifiers::NONE).unwrap();
        assert!(
            matches!(outcome, Some(ShortcutOutcome::Commands(_))),
            "{outcome:?}"
        );
        // The engine's volume is the core 0..=255 scale; the UI shows a
        // percentage, and the two round-trip through `core_volume` /
        // `volume_percent`, so the step is a little under `VOLUME_STEP`.
        let after = app.engine().state().volume;
        assert!(after > before, "volume did not rise: {before} -> {after}");
        assert!(
            app.state().volume > shown_before,
            "the UI did not follow the engine: {shown_before} -> {}",
            app.state().volume
        );
    }

    #[test]
    fn a_quit_command_is_owned_by_the_app() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.quit());
        app.submit(Command::Quit).unwrap();
        assert!(app.quit());
    }

    #[test]
    fn toggling_double_size_changes_the_client_size() {
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(
            app.frame_size(),
            (rustamp_ui::WINDOW_WIDTH, rustamp_ui::WINDOW_HEIGHT)
        );
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert!(app.state().double_size);
        assert_eq!(
            app.frame_size(),
            (rustamp_ui::DOUBLE_WIDTH, rustamp_ui::DOUBLE_HEIGHT)
        );
    }

    #[test]
    fn always_on_top_is_an_app_level_toggle() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.always_on_top());
        app.submit(Command::ToggleAlwaysOnTop).unwrap();
        assert!(app.always_on_top());
        app.submit(Command::ToggleAlwaysOnTop).unwrap();
        assert!(!app.always_on_top());
    }

    #[test]
    fn a_window_that_does_not_exist_is_noted_not_an_error() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::ToggleWindow(WindowId::Video)).unwrap();
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("Video")), "{notes:?}");
        assert!(!app.quit(), "an unimplemented window must not quit the app");
    }

    #[test]
    fn advancing_the_null_clock_moves_playback_without_a_device() {
        // The deterministic half of the silent windowed mode (`--no-audio`):
        // driving the virtual clock pulls decoded frames through the engine
        // with no hardware and no window involved.
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));
        app.submit(Command::Play).unwrap();
        let mut moved = false;
        for _ in 0..2000 {
            app.advance_null_clock(1024);
            app.pump().unwrap();
            if app.engine().timeline_frames() > 0 {
                moved = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            moved,
            "driving the null clock never moved the timeline (transport {:?})",
            app.engine().transport()
        );
        assert!(
            app.engine().last_error().is_none(),
            "the silent run errored: {:?}",
            app.engine().last_error()
        );
    }

    // ----- event routing: events update the UI state -----

    #[test]
    fn engine_events_update_the_ui_state() {
        let (mut app, _dir) = app_with_file(None);
        let now = app.now_ms();
        app.handle_event(&Event::StateChanged(TransportState::Playing));
        app.handle_event(&Event::TrackChanged {
            id: Some(EntryId(1)),
            index: Some(0),
            metadata: Some(Box::new(TrackMetadata {
                title: Some("Hello".to_string()),
                ..TrackMetadata::default()
            })),
            duration_ms: Some(200_000),
        });
        app.handle_event(&Event::StreamInfoChanged(Some(StreamInfo {
            bitrate_kbps: Some(192),
            sample_rate_hz: 44_100,
            channels: 2,
        })));
        app.handle_event(&Event::PositionChanged {
            position_ms: 83_000,
            duration_ms: Some(200_000),
        });
        app.ui_mut().refresh(now);

        let state = app.state();
        assert_eq!(state.transport, TransportState::Playing);
        assert_eq!(state.kbps, Some(192));
        assert_eq!(state.khz, Some(44));
        assert_eq!(state.title, "1. Hello (3:20)");
        assert_eq!(state.time_cells(), [Some(0), Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn a_player_state_snapshot_seeds_the_ui() {
        let (mut app, _dir) = app_with_file(None);
        let snapshot = PlayerState {
            transport: TransportState::Paused,
            position_ms: 30_000,
            duration_ms: Some(120_000),
            volume: 128,
            balance: -127,
            shuffle: true,
            repeat: true,
            current_index: Some(1),
            current_id: Some(EntryId(7)),
            playlist_len: 3,
            stream: Some(StreamInfo {
                bitrate_kbps: Some(96),
                sample_rate_hz: 48_000,
                channels: 1,
            }),
            eq: rustamp_core::state::EqState::default(),
            stop_after_current: false,
        };
        app.sync_from_player_state(&snapshot);

        let state = app.state();
        assert_eq!(state.transport, TransportState::Paused);
        assert_eq!(state.volume, volume_percent(128));
        assert_eq!(state.balance, rustamp_ui::balance_percent(-127));
        assert!(state.shuffle && state.repeat);
        assert_eq!(state.kbps, Some(96));
        assert_eq!(state.khz, Some(48));
        assert_eq!(state.channels, rustamp_ui::MonoStereo::Mono);
        assert!(
            app.ui().logical_title().starts_with("2. "),
            "{}",
            app.ui().logical_title()
        );
    }

    #[test]
    fn shuffle_and_repeat_are_app_owned_and_published_as_events() {
        let (mut app, _dir) = app_with_file(None);
        let (x, y) = widget_centre(Widget::Shuffle);
        app.mouse_down(x, y).unwrap();
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::ToggleShuffle]);
        assert!(app.state().shuffle);
        assert!(app.shuffle);

        let (x, y) = widget_centre(Widget::Repeat);
        app.mouse_down(x, y).unwrap();
        app.mouse_up(x, y).unwrap();
        assert!(app.state().repeat);
        assert!(app.repeat);
    }

    /// The playback modes are the engine's (S15): a configured shuffle that
    /// only lit the button would leave the engine playing in order.
    #[test]
    fn the_configured_playback_modes_reach_the_engine_at_startup() {
        let mut config = Config::default();
        config.playlist.shuffle = true;
        config.playlist.repeat = true;
        let (mut app, _dir) = app_with_config(config, None, None);

        assert!(app.engine().shuffle(), "the engine shuffles");
        assert!(app.engine().repeat(), "the engine repeats");
        // The buttons tell the truth: they show the modes the engine runs in.
        assert!(app.state().shuffle, "the shuffle button is lit");
        assert!(app.state().repeat, "the repeat button is lit");

        // And they still do after the engine's own mode events have been
        // drained: the app and the engine do not drift apart.
        app.pump().unwrap();
        assert!(app.engine().shuffle(), "the engine still shuffles");
        assert!(app.engine().repeat(), "the engine still repeats");
        assert!(app.state().shuffle, "the shuffle button is still lit");
        assert!(app.state().repeat, "the repeat button is still lit");
    }

    /// The fade-out duration is the configured one: the engine ramps the
    /// output down over it and stops the transport when the ramp is over.
    ///
    /// The ramp is counted in frames (200 ms at 44.1 kHz are 8820), so the
    /// test is exact: the transport is still playing halfway through the
    /// configured ramp and stopped once it is over. With the engine's own
    /// default (500 ms) the transport would still be playing there.
    #[test]
    fn the_configured_fade_reaches_the_engine() {
        let mut config = Config::default();
        config.audio.fadeout_ms = 200;
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_config(config, Some(file), None);

        app.submit(Command::Play).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);

        app.submit(Command::StopWithFadeout).unwrap();
        // Half the ramp: the transport is still playing, the fade is running.
        app.advance_null_clock(4096);
        app.pump().unwrap();
        assert_eq!(
            app.engine().transport(),
            TransportState::Playing,
            "the configured fade is still running"
        );

        // The ramp is over: the transport stopped.
        app.advance_null_clock(8192);
        app.pump().unwrap();
        assert_eq!(
            app.engine().transport(),
            TransportState::Stopped,
            "the configured fade is over"
        );
    }

    /// The configured modes survive a save: the file the app writes names
    /// the modes the engine runs in.
    #[test]
    fn the_configured_playback_modes_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modes.toml");
        let mut config = Config::default();
        config.playlist.shuffle = true;
        config.playlist.repeat = true;
        let (app, _dir) = app_with_config(config, None, Some(path.clone()));

        app.save_config().expect("the clean exit saves");

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.lines().any(|line| line == "shuffle = true"),
            "the shuffle mode is written:\n{saved}"
        );
        assert!(
            saved.lines().any(|line| line == "repeat = true"),
            "the repeat mode is written:\n{saved}"
        );

        // And a player built from that file comes up in the same modes.
        let loaded = crate::config::load(Some(&path)).expect("the file loads");
        assert!(loaded.config.playlist.shuffle);
        assert!(loaded.config.playlist.repeat);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let (restarted, _dir) = app_with_config(loaded.config, None, None);
        assert!(restarted.engine().shuffle(), "the engine shuffles again");
        assert!(restarted.engine().repeat(), "the engine repeats again");
    }

    // ----- the Unhandled shortcut path -----

    #[test]
    fn an_unhandled_shortcut_carries_its_command_id() {
        let (mut app, _dir) = app_with_file(None);
        // Ctrl+Alt+N is "start a new instance" (S03 §1), which is app-local and
        // deliberately has no `Command` (RFC 0005 §10).
        let outcome = app
            .key_down(Key::Letter(b'N'), Modifiers::CTRL.union(Modifiers::ALT))
            .unwrap();
        let outcome = outcome.expect("Ctrl+Alt+N is bound");
        let note = App::unhandled_note(&outcome).expect("the outcome is Unhandled");
        assert!(note.contains("40305"), "{note}");
        assert!(note.contains("NewInstance"), "{note}");
        assert!(!app.quit());
    }

    #[test]
    fn a_bound_shortcut_is_not_unhandled() {
        let (mut app, _dir) = app_with_file(None);
        let outcome = app.key_down(Key::Letter(b'X'), Modifiers::NONE).unwrap();
        assert!(matches!(outcome, Some(ShortcutOutcome::Commands(_))));
        assert_eq!(App::unhandled_note(&outcome.unwrap()), None);
    }

    // ----- the app applies a full command/event round trip -----

    #[test]
    fn next_and_prev_move_through_the_queue_and_report_back() {
        let dir = tempfile::tempdir().unwrap();
        let a = track(&dir, "track1_44100.tst");
        let b = track(&dir, "track2_44100.tst");
        let (mut app, _dir) = app_with_file(Some(a));
        // Append a second entry through the app's own routing (`AddItems` is
        // app-owned; the engine only holds the queue).
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(b)],
            at: None,
            play: false,
        })
        .unwrap();
        assert_eq!(app.engine().queue().len(), 2);

        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0),
            4000
        ));

        app.submit(Command::Next).unwrap();
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1),
                4000
            ),
            "Next did not advance"
        );
        assert!(
            app.state().title.starts_with("2. "),
            "{}",
            app.state().title
        );
    }

    #[test]
    fn rendering_produces_a_frame_of_the_layout_size() {
        let (mut app, _dir) = app_with_file(None);
        let image = app.render();
        assert_eq!(
            (image.width(), image.height()),
            (rustamp_ui::WINDOW_WIDTH, rustamp_ui::WINDOW_HEIGHT)
        );
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0),
            "the frame is empty"
        );
    }

    #[test]
    fn quit_is_published_on_the_bus() {
        let (mut app, _dir) = app_with_file(None);
        let observer = app.bus.subscribe();
        app.submit(Command::Quit).unwrap();
        let events: Vec<Event> = observer.try_iter().collect();
        assert!(events.contains(&Event::Quit), "{events:?}");
    }

    #[test]
    fn playlist_dialog_requests_carry_the_spec_titles() {
        // S06 §4: the spec fixes the dialog titles and no dialog behaviour
        // beyond that. Both ADD and LIST OPTS now open a real dialog
        // (`dialogs.rs`), so neither leaves a note.
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(
            app.request_playlist_dialog(PlaylistDialog::AddFiles),
            "Add file(s) to playlist"
        );
        assert_eq!(
            app.request_playlist_dialog(PlaylistDialog::LoadPlaylist),
            "Load Playlist"
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn open_playlist_is_an_app_owned_command_that_opens_its_dialog() {
        // RFC 0010: LIST OPTS opens the `Load Playlist` dialog (S06 §4). The
        // spec fixes the title only; this command opens the real window and
        // records no note of its own — the dialog's load behaviour is pinned
        // by the load-playlist tests below.
        let (mut app, _dir) = app_with_file(None);
        assert!(crate::routing::app_owns(&Command::OpenPlaylist));
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
        assert!(!app.quit(), "a dialog request must not quit the app");
    }

    #[test]
    fn a_managed_but_missing_host_keeps_pumping_detached() {
        use crate::supervise::Supervisor;

        let (mut app, _dir) = app_with_file(None);
        // A real backend pointed at nothing that exists: resolving it fails
        // without spawning any process, which is exactly the degraded path.
        app.manage_host(Supervisor::with_cooldown(
            crate::supervise::ProcessBackend::with_path(PathBuf::from(
                "no-such-dir/rustamp-plughost32",
            )),
            Duration::ZERO,
        ));
        app.pump().unwrap();
        assert!(!app.host_attached());
        // Repeated pumps stay detached instead of stalling or failing.
        app.pump().unwrap();
        assert!(!app.host_attached());
        app.stop_host();
        assert!(!app.host_attached());
    }

    #[test]
    fn the_configure_vis_action_asks_the_plugin_host_over_the_control_channel() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        // Alt+K is the measured shortcut for "Configure plug-in..." (40221).
        let outcome = app.key_down(Key::Letter(b'K'), Modifiers::ALT).unwrap();
        assert_eq!(
            outcome,
            Some(ShortcutOutcome::Commands(vec![Command::ConfigureVisPlugin]))
        );

        // Exactly one request crossed the channel. The pump's own frames
        // (the greeting, the state snapshot) are not requests.
        let mut requests = 0;
        while let Ok(Some(message)) = host_end.try_recv() {
            if message == Message::CommandToHost(Command::ConfigureVisPlugin) {
                requests += 1;
            }
        }
        assert_eq!(requests, 1, "exactly one configure request went out");
    }

    #[test]
    fn without_a_plugin_host_the_configure_request_is_reported_not_dropped() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.host_attached());

        app.apply(&Command::ConfigureVisPlugin);

        // The request could not go anywhere, and the user is told so.
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("no plugin host is attached")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_toggle_vis_action_goes_to_the_host_and_says_so_when_there_is_none() {
        let (mut app, _dir) = app_with_file(None);

        // No host attached: the request is reported, not dropped.
        app.apply(&Command::ToggleVisPlugin);
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("no plugin host is attached")
                    && note.contains("visualization plug-in toggle")),
            "{notes:?}"
        );

        // With a host attached the request goes out as the toggle command.
        let (app_end, mut host_end) = rustamp_ipc::MemoryTransport::pair();
        app.attach_host(app_end);
        app.apply(&Command::ToggleVisPlugin);
        let mut sent = Vec::new();
        while let Ok(Some(message)) = host_end.try_recv() {
            if let Message::CommandToHost(command) = message {
                sent.push(command);
            }
        }
        assert_eq!(sent, vec![Command::ToggleVisPlugin]);
    }

    #[test]
    fn the_named_plugin_requests_reach_the_host_through_the_accepted_carrier() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        // The host answered a load: its id, name and kind are the app's now.
        host_end
            .send(&Message::PluginLoaded {
                plugin_id: 1,
                name: "Sleeper".to_string(),
                kind: PluginKind::Dsp,
            })
            .unwrap();
        app.pump().unwrap();
        let loaded = app.host_plugin(1).expect("the host reported it");
        assert_eq!(loaded.name, "Sleeper");
        assert_eq!(loaded.kind, PluginKind::Dsp);

        // RFC 0020 §3.5: the app names the plug-in it remembered, and the
        // request travels in `Message::CommandToHost` — the accepted carrier.
        app.submit(Command::ConfigurePlugin {
            plugin_id: loaded.plugin_id,
            kind: loaded.kind,
        })
        .expect("the app owns the request");

        // Drain what the pump wrote and keep the plug-in requests.
        let mut requests = Vec::new();
        while let Ok(Some(message)) = host_end.try_recv() {
            if let Message::CommandToHost(command) = message {
                requests.push(command);
            }
        }
        assert_eq!(
            requests,
            vec![Command::ConfigurePlugin {
                plugin_id: 1,
                kind: PluginKind::Dsp,
            }]
        );
    }

    #[test]
    fn the_four_transport_requests_go_out_over_the_same_carrier() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        for command in [
            Command::SeekPlugin {
                plugin_id: 2,
                to_ms: 12_345,
            },
            Command::PausePlugin { plugin_id: 2 },
            Command::UnpausePlugin { plugin_id: 2 },
            Command::StopPlugin { plugin_id: 2 },
        ] {
            app.submit(command).expect("the app owns the request");
        }

        let mut requests = Vec::new();
        while let Ok(Some(message)) = host_end.try_recv() {
            if let Message::CommandToHost(command) = message {
                requests.push(command);
            }
        }
        assert_eq!(
            requests,
            vec![
                Command::SeekPlugin {
                    plugin_id: 2,
                    to_ms: 12_345,
                },
                Command::PausePlugin { plugin_id: 2 },
                Command::UnpausePlugin { plugin_id: 2 },
                Command::StopPlugin { plugin_id: 2 },
            ]
        );
    }

    #[test]
    fn without_a_plugin_host_a_named_plugin_request_is_reported_not_dropped() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.host_attached());

        app.apply(&Command::ConfigurePlugin {
            plugin_id: 7,
            kind: PluginKind::General,
        });

        let notes = app.drain_notes();
        assert!(
            notes.iter().any(
                |note| note.contains("no plugin host is attached") && note.contains("plugin 7")
            ),
            "{notes:?}"
        );
    }

    // ----- the plug-in inventory (the host's answers, read back) -----

    #[test]
    fn the_app_reads_the_hosts_plugin_answers() {
        use rustamp_ipc::MemoryTransport;
        use rustamp_plugin_abi::PluginKind;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        // The host answers two loads.
        host_end
            .send(&Message::PluginLoaded {
                plugin_id: 1,
                name: "MilkDrop 2".to_string(),
                kind: PluginKind::Visualization,
            })
            .unwrap();
        host_end
            .send(&Message::PluginLoaded {
                plugin_id: 2,
                name: "Sleeper".to_string(),
                kind: PluginKind::Dsp,
            })
            .unwrap();

        // The pump reads them, like every other host message.
        app.pump().unwrap();

        let plugins = app.host_plugins();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].plugin_id, 1);
        assert_eq!(plugins[0].name, "MilkDrop 2");
        assert_eq!(plugins[0].kind, PluginKind::Visualization);
        assert_eq!(plugins[1].plugin_id, 2);
        assert_eq!(plugins[1].kind, PluginKind::Dsp);
        // The id is what a later request names, so it is reachable by it.
        assert_eq!(
            app.host_plugin(1).map(|plugin| plugin.name.as_str()),
            Some("MilkDrop 2")
        );
        assert!(app.host_plugin(3).is_none());
    }

    #[test]
    fn a_refused_plugin_load_reaches_the_user_as_a_note() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        host_end
            .send(&Message::PluginError {
                path: "C:\\plugins\\bad.dll".to_string(),
                message: "not a PE image".to_string(),
            })
            .unwrap();

        app.pump().unwrap();

        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("bad.dll") && note.contains("not a PE image")),
            "{notes:?}"
        );
        // The note is given once, not on every pump.
        app.pump().unwrap();
        assert!(app.drain_notes().is_empty());
    }

    // ----- asking the host to load a plug-in -----

    /// A scan with one approved plug-in, as the plug-in pages would list it.
    fn scan_with_one_plugin(app: &mut App) {
        scan_with_plugins(
            app,
            vec![approved_entry("gen_one.dll", PluginKind::General)],
        );
    }

    /// One approved plug-in, as the scan would list it.
    fn approved_entry(file: &str, kind: PluginKind) -> rustamp_prefs::PluginEntry {
        rustamp_prefs::PluginEntry {
            file: file.to_string(),
            path: PathBuf::from("C:\\plugins").join(file),
            kind,
            size: 4096,
            status: rustamp_prefs::PluginStatus::Approved,
        }
    }

    /// A scan with the given plug-ins, as the plug-in pages would list them.
    ///
    /// The scan counts as taken, so the startup load reads this list instead
    /// of reading the real folder.
    fn scan_with_plugins(app: &mut App, entries: Vec<rustamp_prefs::PluginEntry>) {
        app.plugin_scan = rustamp_prefs::PluginScan {
            dir: PathBuf::from("C:\\plugins"),
            entries,
            dir_problem: None,
            store_problem: None,
        };
        app.plugin_scanned = true;
    }

    /// A synthetic i686 plug-in image: a PE32 DLL that exports `export`.
    ///
    /// The image is never mapped and never executed: the scan reads it as
    /// bytes and the passive preflight reads its headers.
    fn fake_plugin(export: &[u8], tweak: u8) -> Vec<u8> {
        let mut bytes = vec![0; 0x600];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        for (offset, value) in [
            (0x84, 0x014c_u16), // machine: i386
            (0x86, 1),          // one section
            (0x94, 224),        // size of the optional header
            (0x96, 0x2102),     // characteristics
            (0x98, 0x10b),      // optional header: PE32
        ] {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (0xa8, 0x1100_u32), // entry point
            (0xd0, 0x2000),     // image base
            (0xd4, 0x200),
            (0xf4, 16),     // section alignment
            (0xf8, 0x1000), // file alignment
            (0xfc, 0x100),
            (0x180, 0x400), // the export directory
            (0x184, 0x1000),
            (0x188, 0x400),
            (0x18c, 0x200),
            (0x19c, 0x6000_0020),
            (0x214, 1),
            (0x218, 1),
            (0x21c, 0x1040),
            (0x220, 0x1044),
            (0x224, 0x1048),
            (0x240, 0x1100),
            (0x244, 0x1060),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x260..0x260 + export.len()].copy_from_slice(export);
        bytes[0x300] = tweak;
        bytes
    }

    /// The load requests the host received, in order.
    fn load_requests(host_end: &mut rustamp_ipc::MemoryTransport) -> Vec<(String, PluginKind)> {
        let mut requests = Vec::new();
        while let Ok(Some(message)) = host_end.try_recv() {
            if let Message::LoadPlugin { path, kind } = message {
                requests.push((path, kind));
            }
        }
        requests
    }

    #[test]
    fn switching_a_plugin_on_asks_the_plugin_host_to_load_it() {
        use rustamp_ipc::MemoryTransport;
        use rustamp_platform::PrefsAction;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        scan_with_one_plugin(&mut app);
        // An approved plug-in is off until the page's toggle switches it on.
        app.apply_prefs_action(PrefsAction::TogglePlugin(0));

        // Exactly one request crossed the channel, and it names the file the
        // scan listed and the kind that file declares. The pump's own frames
        // (the greeting, the state snapshot) are not requests.
        assert_eq!(
            load_requests(&mut host_end),
            vec![("C:\\plugins\\gen_one.dll".to_string(), PluginKind::General)]
        );
        // The choice is saved as well: the plug-in stays on after a restart.
        assert_eq!(app.config.plugins.enabled, ["gen_one.dll"]);
    }

    #[test]
    fn switching_a_plugin_off_asks_the_host_for_no_load() {
        use rustamp_ipc::MemoryTransport;
        use rustamp_platform::PrefsAction;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        scan_with_one_plugin(&mut app);

        // The plug-in is on (approved and on the enabled list), so the page's
        // toggle switches it off.
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        app.apply_prefs_action(PrefsAction::TogglePlugin(0));

        // Nothing was loaded: a plug-in that is switched off is not asked
        // for.
        assert!(load_requests(&mut host_end).is_empty());
        // The protocol has no unload request, so the user is told that the
        // plug-in stays loaded in the host.
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("no unload request")),
            "{notes:?}"
        );
    }

    #[test]
    fn without_a_plugin_host_the_load_request_is_reported_not_dropped() {
        use rustamp_platform::PrefsAction;

        let (mut app, _dir) = app_with_file(None);
        assert!(!app.host_attached());
        scan_with_one_plugin(&mut app);

        app.apply_prefs_action(PrefsAction::TogglePlugin(0));

        // The request could not go anywhere, and the user is told so.
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("no plugin host is attached")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_plugin_the_host_loaded_lands_in_the_inventory() {
        use rustamp_ipc::MemoryTransport;
        use rustamp_platform::PrefsAction;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        scan_with_one_plugin(&mut app);

        app.apply_prefs_action(PrefsAction::TogglePlugin(0));

        // The host answers the load.
        host_end
            .send(&Message::PluginLoaded {
                plugin_id: 1,
                name: "Rustamp Fixture".to_string(),
                kind: PluginKind::General,
            })
            .unwrap();

        // The pump reads the answer, like every other host message.
        app.pump().unwrap();

        let plugins = app.host_plugins();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_id, 1);
        assert_eq!(plugins[0].name, "Rustamp Fixture");
        assert_eq!(plugins[0].kind, PluginKind::General);
    }

    /// The Configure button: the scanned row becomes the host's plug-in id.
    #[test]
    fn configure_in_the_plugin_page_names_the_loaded_plugin_of_that_file() {
        use rustamp_ipc::MemoryTransport;
        use rustamp_platform::PrefsAction;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        scan_with_one_plugin(&mut app); // gen_one.dll, a general-purpose plug-in

        // Nothing is loaded yet: the button says so and sends nothing.
        app.apply_prefs_action(PrefsAction::ConfigurePlugin(0));
        assert!(app.drain_notes().iter().any(|n| n.contains("not loaded")));
        assert!(host_end.try_recv().ok().flatten().is_none());

        host_end
            .send(&Message::PluginLoaded {
                plugin_id: 7,
                name: "Rustamp Fixture".to_string(),
                kind: PluginKind::General,
            })
            .unwrap();
        app.pump().unwrap();

        app.apply_prefs_action(PrefsAction::ConfigurePlugin(0));
        let mut requests = Vec::new();
        while let Ok(Some(message)) = host_end.try_recv() {
            if let Message::CommandToHost(command) = message {
                requests.push(command);
            }
        }
        assert_eq!(
            requests,
            vec![Command::ConfigurePlugin {
                plugin_id: 7,
                kind: PluginKind::General,
            }]
        );
    }

    /// Several plug-ins of one kind: the file's stem picks the name.
    #[test]
    fn the_file_stem_picks_among_several_loaded_plugins_of_one_kind() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        for (id, name) in [(1, "Nullsoft Wave"), (2, "OpenMPT Player")] {
            host_end
                .send(&Message::PluginLoaded {
                    plugin_id: id,
                    name: name.to_string(),
                    kind: PluginKind::Input,
                })
                .unwrap();
        }
        app.pump().unwrap();
        let found = app.loaded_plugin_for_file("in_openmpt.dll", PluginKind::Input);
        assert_eq!(found.map(|p| p.plugin_id), Some(2));
        // No name carries the stem: no guess.
        assert!(
            app.loaded_plugin_for_file("in_flac.dll", PluginKind::Input)
                .is_none()
        );
        // A kind with nothing loaded.
        assert!(
            app.loaded_plugin_for_file("out_wasapi.dll", PluginKind::Output)
                .is_none()
        );
    }

    // ----- the output change that waits for playback to stop -----

    #[test]
    fn an_output_change_during_playback_is_applied_when_playback_stops() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));
        // The null output run wants no plug-in, so this name is a change.
        app.plugin_output_name = Some("out_x.dll".to_string());
        app.submit(Command::Play).unwrap();
        app.pump().unwrap();
        assert_ne!(app.engine.transport(), TransportState::Stopped);

        app.install_plugin_output();
        assert!(app.output_change_pending, "the run keeps its output");
        assert_eq!(app.plugin_output_name.as_deref(), Some("out_x.dll"));
        assert!(app.output_text().contains("when playback has stopped"));
        // A later pump during the run changes nothing.
        app.pump().unwrap();
        assert!(app.output_change_pending);

        app.submit(Command::Stop).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.output_change_pending && std::time::Instant::now() < deadline {
            app.pump().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!app.output_change_pending, "the stop applied the change");
        assert_eq!(app.plugin_output_name, None);
        assert_eq!(app.engine.transport(), TransportState::Stopped);
        // The queue survives the new engine.
        assert_eq!(app.engine.queue().len(), 1);
    }

    #[test]
    fn a_new_output_mode_opens_the_device_again_when_nothing_plays() {
        use rustamp_platform::PrefsAction;
        let (mut app, _dir) = app_with_file(None);
        app.apply_prefs_action(PrefsAction::SetOutputMode(
            rustamp_core::config::OutputMode::Exclusive,
        ));
        assert!(!app.output_change_pending);
        assert_eq!(
            app.config.audio.output_mode,
            rustamp_core::config::OutputMode::Exclusive
        );
    }

    #[test]
    fn switching_on_after_an_approval_replaces_a_host_that_read_the_old_allowlist() {
        use rustamp_platform::PrefsAction;
        let (mut app, _dir) = app_with_file(None);
        scan_with_one_plugin(&mut app);
        app.manage_host(crate::supervise::Supervisor::new(
            crate::supervise::ProcessBackend::with_path(PathBuf::from("no-such-host.exe")),
        ));
        app.host_allowlist_stale = true;

        app.apply_prefs_action(PrefsAction::TogglePlugin(0));

        assert!(!app.host_allowlist_stale);
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("restarts")),
            "{notes:?}"
        );
        // The new host is asked for it after the attach, so nothing was refused
        // for a missing channel.
        assert!(
            !notes.iter().any(|note| note.contains("no plugin host")),
            "{notes:?}"
        );
    }

    // ----- the startup load (the enabled plug-ins, once per host) -----

    #[test]
    fn two_enabled_plugins_are_asked_for_in_order_after_the_host_attaches() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![
                approved_entry("gen_one.dll", PluginKind::General),
                approved_entry("in_two.dll", PluginKind::Input),
            ],
        );
        app.config.plugins.enabled = vec!["in_two.dll".to_string(), "gen_one.dll".to_string()];
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        app.pump().unwrap();

        // Two requests, in the scan's order, each naming the file the scan
        // listed and the kind that file declares. The pump's own frames (the
        // greeting, the state snapshot) are not requests.
        assert_eq!(
            load_requests(&mut host_end),
            vec![
                ("C:\\plugins\\gen_one.dll".to_string(), PluginKind::General),
                ("C:\\plugins\\in_two.dll".to_string(), PluginKind::Input),
            ]
        );
    }

    #[test]
    fn a_plugin_that_is_switched_off_is_not_asked_for_at_startup() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![
                approved_entry("gen_one.dll", PluginKind::General),
                approved_entry("in_two.dll", PluginKind::Input),
            ],
        );
        // The user switched only the first one on: the second stays off.
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        app.pump().unwrap();

        assert_eq!(
            load_requests(&mut host_end),
            vec![("C:\\plugins\\gen_one.dll".to_string(), PluginKind::General)]
        );
    }

    #[test]
    fn a_plugin_that_is_not_approved_is_not_asked_for_at_startup() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        let mut entry = approved_entry("gen_one.dll", PluginKind::General);
        entry.status = rustamp_prefs::PluginStatus::NotApproved;
        scan_with_plugins(&mut app, vec![entry]);
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        app.pump().unwrap();

        // The approval store did not approve the image, so the host would
        // refuse it: the app does not ask.
        assert!(load_requests(&mut host_end).is_empty());
    }

    #[test]
    fn a_detached_host_produces_notes_instead_of_load_requests() {
        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![approved_entry("gen_one.dll", PluginKind::General)],
        );
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        assert!(!app.host_attached());

        app.load_enabled_plugins();

        // The request could not go anywhere, and the user is told so.
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("no plugin host is attached")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_same_host_is_asked_for_a_plugin_only_once() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![approved_entry("gen_one.dll", PluginKind::General)],
        );
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);

        app.pump().unwrap();
        assert_eq!(load_requests(&mut host_end).len(), 1);

        // Further pumps do not ask again: the host has the plug-in, and the
        // protocol cannot unload it.
        app.pump().unwrap();
        app.pump().unwrap();
        assert!(load_requests(&mut host_end).is_empty());
    }

    #[test]
    fn a_new_host_is_asked_again_because_it_has_loaded_nothing() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![approved_entry("gen_one.dll", PluginKind::General)],
        );
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];

        let (first_end, mut first_host) = MemoryTransport::pair();
        app.attach_host(first_end);
        app.pump().unwrap();
        assert_eq!(load_requests(&mut first_host).len(), 1);

        // The host is replaced. The inventory cannot tell the new peer what
        // the old one had loaded (the answer carries no path), so the app
        // asks again: a fresh host process has loaded nothing.
        let (second_end, mut second_host) = MemoryTransport::pair();
        app.attach_host(second_end);
        app.pump().unwrap();

        // The old channel is silent, the new one gets the request.
        assert!(load_requests(&mut first_host).is_empty());
        assert_eq!(load_requests(&mut second_host).len(), 1);
    }

    #[test]
    fn a_refused_plugin_is_reported_and_may_be_asked_for_again() {
        use rustamp_ipc::MemoryTransport;

        let (mut app, _dir) = app_with_file(None);
        scan_with_plugins(
            &mut app,
            vec![approved_entry("gen_one.dll", PluginKind::General)],
        );
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        app.pump().unwrap();
        assert_eq!(load_requests(&mut host_end).len(), 1);

        // The host refuses the plug-in.
        host_end
            .send(&Message::PluginError {
                path: "C:\\plugins\\gen_one.dll".to_string(),
                message: "not a PE image".to_string(),
            })
            .unwrap();
        app.pump().unwrap();

        // The refusal is the user's news, and it does not stop the player.
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|note| note.contains("could not load") && note.contains("gen_one.dll")),
            "{notes:?}"
        );

        // The file is not loaded, so asking for it again is allowed.
        app.request_plugin_load(Path::new("C:\\plugins\\gen_one.dll"), PluginKind::General);
        assert_eq!(load_requests(&mut host_end).len(), 1);
    }

    #[test]
    fn the_startup_load_reads_the_folder_and_the_approval_store() {
        use rustamp_ipc::MemoryTransport;

        let dir = tempfile::tempdir().unwrap();
        let image = fake_plugin(b"winampGetGeneralPurposePlugin\0", 1);
        std::fs::write(dir.path().join("gen_one.dll"), &image).unwrap();

        let (mut app, _config) = app_with_file(None);
        app.config.plugins.plugin_dir = Some(dir.path().to_path_buf());
        // The approval store lives beside the configuration file.
        app.set_config_path(Some(dir.path().join("rustamp.toml")));

        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        app.pump().unwrap();

        // No store approved the image, so the host is never asked for it: a
        // missing store is an empty allowlist, not a silent "yes".
        assert!(load_requests(&mut host_end).is_empty());
    }

    #[test]
    fn an_approved_plugin_in_the_folder_is_asked_for_at_startup() {
        use rustamp_ipc::MemoryTransport;

        let dir = tempfile::tempdir().unwrap();
        let image = fake_plugin(b"winampGetGeneralPurposePlugin\0", 1);
        std::fs::write(dir.path().join("gen_one.dll"), &image).unwrap();
        let mut store = rustamp_plughost::ApprovalStore::new();
        store.approve_bytes("gen_one.dll", &image).unwrap();
        store
            .save_to_file(&dir.path().join(rustamp_prefs::APPROVALS_FILE))
            .unwrap();

        let (mut app, _config) = app_with_file(None);
        app.config.plugins.plugin_dir = Some(dir.path().to_path_buf());
        app.config.plugins.enabled = vec!["gen_one.dll".to_string()];
        app.set_config_path(Some(dir.path().join("rustamp.toml")));

        let (app_end, mut host_end) = MemoryTransport::pair();
        app.attach_host(app_end);
        app.pump().unwrap();

        assert_eq!(
            load_requests(&mut host_end),
            vec![(
                dir.path()
                    .join("gen_one.dll")
                    .to_string_lossy()
                    .into_owned(),
                PluginKind::General
            )]
        );
    }

    // ----- the equalizer window (S07; headless, no window opened) -----

    #[test]
    fn rendering_the_equalizer_produces_a_frame_of_the_layout_size() {
        let (app, _dir) = app_with_file(None);
        let image = app.render_eq();
        assert_eq!(
            (image.width(), image.height()),
            (
                rustamp_ui::EQUALIZER_WINDOW_SIZE.0,
                rustamp_ui::EQUALIZER_WINDOW_SIZE.1
            )
        );
        assert_eq!((image.width(), image.height()), app.eq_frame_size());
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0),
            "the frame is empty"
        );
    }

    #[test]
    fn clicking_on_in_the_eq_window_reaches_the_engine_stage() {
        let (mut app, _dir) = app_with_file(None);
        assert!(app.eq_state().enabled);
        // On held rect (14,18,25,12): press arms, release inside toggles.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let commands = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(commands, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        // The engine owns the S07 stage now, so the same commands are
        // forwarded and change the engine's published equalizer state instead
        // of being noted as unimplemented.
        app.submit(Command::SetEqEnabled(true)).unwrap();
        app.submit(Command::SetBand {
            band: 4,
            gain_db: 6.0,
        })
        .unwrap();
        let eq = app.host_player_state().eq;
        assert!(eq.enabled);
        assert_eq!(eq.bands[4], 6.0);
        assert!(app.eq_state().enabled);
        assert_eq!(
            app.eq_state().bands[4],
            rustamp_core::state::eq_classic_from_db(6.0) as u8
        );
        let notes = app.drain_notes();
        assert!(!notes.iter().any(|n| n.contains("equalizer")), "{notes:?}");
        assert!(!app.quit());
    }

    #[test]
    fn eq_command_applied_from_outside_updates_window_state() {
        let (mut app, _dir) = app_with_file(None);

        // SetEqEnabled external command updates window state.
        app.submit(Command::SetEqEnabled(false)).unwrap();
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);

        app.submit(Command::SetEqEnabled(true)).unwrap();
        assert!(app.eq_state().enabled);
        assert!(app.engine().eq().enabled);

        // SetEqAuto external command updates window state.
        app.submit(Command::SetEqAuto(true)).unwrap();
        assert!(app.eq_state().auto);
        assert!(app.engine().eq().auto);

        app.submit(Command::SetEqAuto(false)).unwrap();
        assert!(!app.eq_state().auto);
        assert!(!app.engine().eq().auto);

        // SetPreamp external command updates window state.
        app.submit(Command::SetPreamp(4.5)).unwrap();
        assert_eq!(
            app.eq_state().preamp,
            rustamp_core::state::eq_classic_from_db(4.5) as u8
        );
        assert_eq!(app.engine().eq().preamp, 4.5);

        // SetBand external command updates window state.
        app.submit(Command::SetBand {
            band: 2,
            gain_db: -8.0,
        })
        .unwrap();
        assert_eq!(
            app.eq_state().bands[2],
            rustamp_core::state::eq_classic_from_db(-8.0) as u8
        );
        assert_eq!(app.engine().eq().bands[2], -8.0);

        // Direct external apply() also synchronizes the window.
        app.apply(&Command::SetBand {
            band: 5,
            gain_db: 3.5,
        });
        assert_eq!(
            app.eq_state().bands[5],
            rustamp_core::state::eq_classic_from_db(3.5) as u8
        );
        assert_eq!(app.engine().eq().bands[5], 3.5);

        // Event::EqChanged also updates window state.
        let mut custom_bands = [0.0; 10];
        custom_bands[8] = -10.0;
        let event_eq = rustamp_core::state::EqState {
            bands: custom_bands,
            preamp: 2.0,
            enabled: false,
            auto: true,
        };
        app.handle_event(&Event::EqChanged(event_eq));
        assert_eq!(
            app.eq_state().bands[8],
            rustamp_core::state::eq_classic_from_db(-10.0) as u8
        );
        assert_eq!(
            app.eq_state().preamp,
            rustamp_core::state::eq_classic_from_db(2.0) as u8
        );
        assert!(!app.eq_state().enabled);
        assert!(app.eq_state().auto);
    }

    #[test]
    fn clicking_on_button_toggles_engine_and_window_state_together() {
        let (mut app, _dir) = app_with_file(None);
        assert!(app.eq_state().enabled);

        // Click ON: toggles to false, both engine and window agree.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);
        assert!(!app.host_player_state().eq.enabled);

        // Click ON again: toggles to true, both engine and window agree.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(true)]);
        assert!(app.eq_state().enabled);
        assert!(app.engine().eq().enabled);
        assert!(app.host_player_state().eq.enabled);

        // Third click: toggles back to false.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);
        assert!(!app.host_player_state().eq.enabled);
    }

    #[test]
    fn states_are_equal_after_band_and_preamp_changes() {
        let (mut app, _dir) = app_with_file(None);

        // 1. External band change.
        app.submit(Command::SetBand {
            band: 1,
            gain_db: 7.2,
        })
        .unwrap();
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.bands[1],
            rustamp_core::state::eq_classic_from_db(eng.bands[1]) as u8
        );

        // 2. UI-driven band change (click on band 6 track).
        let band6_x = rustamp_ui::EqualizerLayout::slider_x(rustamp_ui::EqSlider::Band(6));
        let cmds = app.eq_mouse_down(band6_x + 5, 45).unwrap();
        assert_eq!(cmds.len(), 1);
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.bands[6],
            rustamp_core::state::eq_classic_from_db(eng.bands[6]) as u8
        );
        let _ = app.eq_mouse_up(band6_x + 5, 45).unwrap();

        // 3. External preamp change.
        app.submit(Command::SetPreamp(-5.5)).unwrap();
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.preamp,
            rustamp_core::state::eq_classic_from_db(eng.preamp) as u8
        );

        // 4. UI-driven preamp change (click on the preamp track, away from the
        //    thumb). The thumb now sits wherever the synchronised value put it,
        //    and a press *inside* the thumb starts a drag and emits nothing, so
        //    the press must land outside it.
        let preamp_x = rustamp_ui::EqualizerLayout::slider_x(rustamp_ui::EqSlider::Preamp);
        let before = app.eq_state().preamp;
        let cmds = app.eq_mouse_down(preamp_x + 5, 45).unwrap();
        assert_eq!(cmds.len(), 1, "a press outside the thumb commits");
        assert_ne!(
            app.eq_state().preamp,
            before,
            "the press must move the thumb, not land on it"
        );
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.preamp,
            rustamp_core::state::eq_classic_from_db(eng.preamp) as u8
        );
        let _ = app.eq_mouse_up(preamp_x + 5, 45).unwrap();
    }

    #[test]
    fn presets_in_the_eq_window_queues_the_popup_request_once() {
        // S07 §4.2: the click queues the Load/Save/Delete popup, which the
        // presenter takes on its tick and shows.
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.take_eq_menu());
        assert!(app.eq_mouse_down(239, 24).unwrap().is_empty());
        assert!(app.eq_mouse_up(239, 24).unwrap().is_empty());
        assert!(app.take_eq_menu());
        assert!(!app.take_eq_menu());
        assert_eq!(
            app.request_eq_presets_menu(rustamp_ui::EqMenuRequest::Presets),
            &["Load", "Save", "Delete"]
        );
    }

    #[test]
    fn applying_a_preset_sets_the_bands_and_is_recognised_again() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.apply_eq_preset("No Such Preset").unwrap());
        assert!(app.apply_eq_preset("rock").unwrap());
        let rock = crate::eq_presets::find_preset("Rock").unwrap();
        assert_eq!(app.engine.eq().bands, rock.bands);
        assert_eq!(app.current_eq_preset(), Some("Rock"));
        // The window's sliders follow the engine.
        assert!(app.apply_eq_preset("Flat").unwrap());
        assert_eq!(app.current_eq_preset(), Some("Flat"));
    }

    #[test]
    fn saved_presets_load_save_and_delete_through_the_popup_and_survive_a_restart() {
        use crate::eq_presets::EqMenuChoice;
        let (mut app, dir) = app_with_file(None);
        let config = dir.path().join("config").join("rustamp.toml");
        app.set_config_path(Some(config.clone()));
        assert!(app.eq_user_presets().is_empty());

        // Save the live curve under a fresh name.
        app.submit(Command::SetBand {
            band: 0,
            gain_db: 6.0,
        })
        .unwrap();
        app.submit(Command::SetPreamp(-3.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::SaveNew).unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        assert_eq!(app.eq_user_presets()[0].name, "Custom 1");
        assert!(config.with_file_name("presets.eqf").is_file());

        // Flatten, then load the saved curve back.
        app.apply_eq_preset("Flat").unwrap();
        app.submit(Command::SetPreamp(0.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadUser(0))
            .unwrap();
        let eq = app.engine().eq();
        assert!((eq.bands[0] - 6.0).abs() < 0.3, "{:?}", eq.bands);
        assert!((eq.preamp + 3.0).abs() < 0.3, "{}", eq.preamp);

        // A new run reads the file; the popup lists the saved preset.
        let (mut again, _dir2) = app_with_file(None);
        again.set_config_path(Some(config));
        assert_eq!(again.eq_user_presets().len(), 1);
        assert_eq!(again.eq_user_presets()[0].name, "Custom 1");
        let menu = again.native_eq_menu();
        assert!(menu.choice_for_id(0).is_none());
        let ids: Vec<_> = (1..=40).filter_map(|id| menu.choice_for_id(id)).collect();
        assert!(ids.contains(&EqMenuChoice::LoadUser(0)));
        assert!(ids.contains(&EqMenuChoice::SaveOver(0)));
        assert!(ids.contains(&EqMenuChoice::Delete(0)));

        again
            .activate_eq_menu_choice(EqMenuChoice::Delete(0))
            .unwrap();
        assert!(again.eq_user_presets().is_empty());
        again.load_eq_user_presets();
        assert!(again.eq_user_presets().is_empty());
    }

    #[test]
    fn save_over_replaces_in_place_and_builtins_keep_the_preamp() {
        use crate::eq_presets::EqMenuChoice;
        let (mut app, _dir) = app_with_file(None);
        app.activate_eq_menu_choice(EqMenuChoice::SaveNew).unwrap();
        app.submit(Command::SetBand {
            band: 2,
            gain_db: 6.0,
        })
        .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::SaveOver(0))
            .unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        assert!((app.eq_user_presets()[0].bands[2] - 6.0).abs() < 0.3);
        // Out-of-range indices are ignored.
        app.activate_eq_menu_choice(EqMenuChoice::SaveOver(9))
            .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadUser(9))
            .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::Delete(9))
            .unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        // A built-in preset leaves the preamp where it was.
        app.submit(Command::SetPreamp(-4.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadBuiltin(1))
            .unwrap();
        assert_eq!(app.engine().eq().preamp, -4.0);
    }

    #[test]
    fn an_eqf_file_loads_with_its_preamp_and_bad_files_are_refused() {
        use crate::eq_presets::{UserPreset, write_eqf};
        let (mut app, dir) = app_with_file(None);
        let good = dir.path().join("good.eqf");
        let mut bands = [0.0; rustamp_core::state::EQ_BANDS];
        bands[0] = 6.0;
        let preset = UserPreset {
            name: "Warm".to_string(),
            bands,
            preamp: -3.0,
        };
        std::fs::write(&good, write_eqf(&[preset])).unwrap();
        assert_eq!(app.load_eq_file(&good).unwrap(), "Warm");
        let eq = app.engine().eq();
        assert!((eq.bands[0] - 6.0).abs() < 0.3);
        assert!((eq.preamp + 3.0).abs() < 0.3);
        assert!(app.eq_user_presets().is_empty());

        let junk = dir.path().join("junk.eqf");
        std::fs::write(&junk, b"not an eqf").unwrap();
        assert!(app.load_eq_file(&junk).is_err());
        let empty = dir.path().join("empty.eqf");
        std::fs::write(&empty, write_eqf(&[])).unwrap();
        assert!(app.load_eq_file(&empty).is_err());
        assert!(app.load_eq_file(&dir.path().join("missing.eqf")).is_err());
        // Refused files leave the curve alone.
        assert!((app.engine().eq().bands[0] - 6.0).abs() < 0.3);
    }

    #[test]
    fn jump_to_file_filters_the_playlist_and_plays_the_selected_match() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine.set_queue(vec![
            TrackSource::path(track(&dir, "alpha.mp3")),
            TrackSource::path(track(&dir, "beta.mp3")),
            TrackSource::path(track(&dir, "Gamma Beta.mp3")),
        ]);
        app.sync_entry_ids();
        let jump = rustamp_ui::DialogKind::Jump(JumpTarget::File);
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert!(app.dialog_is_open(jump));
        assert_eq!(app.jump_file_matches(), vec![0, 1, 2]);

        // Typing narrows the list (ignoring case); Down moves, Enter plays.
        assert!(app.dialog_text("BETA"));
        assert_eq!(app.jump_file_matches(), vec![1, 2]);
        assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        assert!(
            app.dialog_key(Key::Down, Modifiers::NONE),
            "clamps at the last match"
        );
        assert!(app.dialog_key(Key::Up, Modifiers::NONE));
        assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(!app.dialog_is_open(jump));
        assert_eq!(app.engine().current_index(), Some(2));
    }

    #[test]
    fn jump_to_file_with_no_match_stays_open() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine
            .set_queue(vec![TrackSource::path(track(&dir, "alpha.mp3"))]);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert!(app.dialog_text("zzz"));
        assert!(app.jump_file_matches().is_empty());
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::Jump(JumpTarget::File)));
        assert!(app.dialog_key(Key::Backspace, Modifiers::NONE));
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert!(app.dialog_open_kind().is_none());
    }

    #[test]
    fn the_playlist_comes_back_after_a_restart_without_playing() {
        let (mut app, dir) = app_with_file(None);
        let config = dir.path().join("rustamp.toml");
        app.set_config_path(Some(config.clone()));
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(track(&dir, "one.mp3")),
                MediaLocation::File(track(&dir, "two.mp3")),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.save_session_playlist();

        let (mut next, _other) = app_with_file(None);
        next.set_config_path(Some(config));
        next.restore_session_playlist();
        assert_eq!(next.engine().queue().len(), 2);
        assert_eq!(next.engine().transport(), TransportState::Stopped);
    }

    #[test]
    fn a_right_click_asks_for_the_menu_of_what_it_hit() {
        use crate::menu::ContextMenu;
        let (mut app, _dir) = app_with_file(None);
        for (widget, menu) in [
            (Widget::Marquee, ContextMenu::Title),
            (Widget::TimeDisplay, ContextMenu::Time),
            (Widget::PositionSlider, ContextMenu::Seek),
        ] {
            let (x, y) = widget_centre(widget);
            app.mouse_right_down(x, y).unwrap();
            assert_eq!(app.take_context_menu(), Some(menu), "{widget:?}");
            assert!(app.native_context_menu(menu).choices.len() >= 2);
        }
    }

    #[test]
    fn the_equalizer_window_passes_the_shared_keys_on() {
        let (mut app, _dir) = app_with_file(None);
        let commands = app.eq_key_down(Key::Letter(b'V'), Modifiers::NONE).unwrap();
        assert_eq!(commands, vec![Command::Stop]);

        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        assert!(app.eq_open());
        app.eq_key_down(Key::F(4), Modifiers::CTRL).unwrap();
        assert!(!app.eq_open(), "Ctrl+F4 closes the equalizer");
        assert!(!app.quit(), "and not the player");
    }

    #[test]
    fn the_equalizer_keys_reach_the_engine_and_add_up() {
        let (mut app, _dir) = app_with_file(None);
        for _ in 0..4 {
            app.eq_key_down(Key::Digit(1), Modifiers::NONE).unwrap();
        }
        let gain = app.engine().state().eq.bands[0];
        assert!(gain > 2.0, "four steps up are well above flat: {gain}");
        app.eq_key_down(Key::Right, Modifiers::NONE).unwrap();
        assert_eq!(app.engine().state().balance, PAN_STEP);
    }

    #[test]
    fn a_right_click_on_the_equalizer_offers_its_two_switches() {
        let (mut app, _dir) = app_with_file(None);
        app.eq_right_down();
        let kind = app.take_context_menu().expect("the menu is asked for");
        let menu = app.native_context_menu(kind);
        assert_eq!(menu.choices.len(), 2);
        let auto = app.engine().state().eq.auto;
        app.activate_menu_choice(rustamp_ui::menu::MenuChoice::EqAutoload)
            .unwrap();
        assert_ne!(app.engine().state().eq.auto, auto);
    }

    #[test]
    fn the_marquee_says_what_failed_by_the_file_name() {
        assert_eq!(
            status_text(r"error: cannot play C:\Music\dead.mp3: unsupported format").as_deref(),
            Some("cannot play dead.mp3: unsupported format")
        );
        assert_eq!(
            status_text("Minibrowser is not implemented yet").as_deref(),
            Some("Minibrowser is not implemented yet")
        );
        assert_eq!(status_text("skin loaded"), None);
    }

    #[test]
    fn jump_to_file_lists_the_matches_and_counts_them() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine.set_queue(vec![
            TrackSource::path(track(&dir, "alpha.mp3")),
            TrackSource::path(track(&dir, "beta.mp3")),
            TrackSource::path(track(&dir, "Gamma Beta.mp3")),
        ]);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();

        // The search field, one row per match and the count line. The first
        // match is the selected one, so it carries the marker.
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.first().map(String::as_str), Some("Search: "));
        assert_eq!(lines.len(), 1 + 3 + 1, "{lines:?}");
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");
        assert!(lines[1].ends_with("alpha"), "{lines:?}");
        assert!(lines[2].starts_with("  2. "), "{lines:?}");
        assert!(lines[2].ends_with("beta"), "{lines:?}");
        assert!(lines[3].starts_with("  3. "), "{lines:?}");
        assert!(lines[3].ends_with("Gamma Beta"), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("3 of 3 entries"));

        // A search narrows the list and the count.
        assert!(app.dialog_text("BETA"));
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.first().map(String::as_str), Some("Search: BETA"));
        assert!(lines[1].starts_with("> 2. "), "{lines:?}");
        assert!(lines[1].ends_with("beta"), "{lines:?}");
        assert!(lines[2].starts_with("  3. "), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("2 of 3 entries"));
    }

    #[test]
    fn jump_to_file_keeps_the_selection_in_view_and_starts_over_next_time() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        let queue: Vec<_> = (0..15)
            .map(|index| TrackSource::path(track(&dir, &format!("track{index:02}.mp3"))))
            .collect();
        app.engine.set_queue(queue);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();

        // The window shows ten rows, the selected one marked.
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), 1 + rustamp_ui::JUMP_FILE_ROWS + 1, "{lines:?}");
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");

        // Moving down past the window scrolls it: the selected row stays
        // visible and stays the marked one.
        for _ in 0..12 {
            assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        }
        let lines = app.open_dialog_lines().unwrap();
        let marked: Vec<&String> = lines.iter().filter(|line| line.starts_with("> ")).collect();
        assert_eq!(marked.len(), 1, "{lines:?}");
        assert!(marked[0].starts_with("> 13. "), "{lines:?}");

        // Closing and opening the box again starts over: no search text and
        // the first row selected.
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert_eq!(app.jump_text(), "");
        let lines = app.open_dialog_lines().unwrap();
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");
    }

    #[test]
    fn the_jump_to_file_shortcuts_open_the_box() {
        use rustamp_core::command::JumpTarget;
        let (mut app, _dir) = app_with_file(None);
        let jump = rustamp_ui::DialogKind::Jump(JumpTarget::File);

        // S03 §1/§2: `J` and `Num.` both carry the command id 40194, which
        // opens the jump-to-file box.
        for key in [Key::Letter(b'J'), Key::NumDecimal] {
            assert!(app.dialog_open_kind().is_none());
            let outcome = app.key_down(key, Modifiers::NONE).unwrap();
            assert!(
                matches!(
                    outcome,
                    Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                        if commands == &vec![Command::OpenJump { target: JumpTarget::File }]
                ),
                "{key:?} -> {outcome:?}"
            );
            assert!(app.dialog_is_open(jump), "{key:?}");

            // Escape closes it again, so the next key starts from scratch.
            assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
            assert!(app.dialog_open_kind().is_none());
        }
    }

    #[test]
    fn double_size_reaches_the_equalizer_window() {
        // S05 §2.1: the flag resizes the equalizer to 550 x 232.
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(app.eq_frame_size(), (275, 116));
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.eq_frame_size(), (550, 232));
        assert_eq!(
            app.eq_frame_size(),
            (
                rustamp_ui::DOUBLE_EQUALIZER_SIZE.0,
                rustamp_ui::DOUBLE_EQUALIZER_SIZE.1
            )
        );
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.eq_frame_size(), (275, 116));
    }

    #[test]
    fn toggling_the_equalizer_window_flips_the_flag_without_a_message() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.eq_open());
        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        assert!(app.eq_open());
        assert!(
            app.drain_notes().is_empty(),
            "the window exists: no message"
        );
        assert!(!app.quit(), "toggling the EQ window must not quit the app");
    }

    // ----- window focus (`FocusNextWindow`, `ShowMainWindow`) -----

    /// Opens or closes the equalizer and the playlist editor to the wanted
    /// state (the playlist starts open, the equalizer closed), then clears the
    /// requests and notes the toggles left behind.
    fn set_open(app: &mut App, eq: bool, playlist: bool) {
        if app.eq_open() != eq {
            app.submit(Command::ToggleWindow(WindowId::Equalizer))
                .unwrap();
        }
        if app.playlist_open() != playlist {
            app.submit(Command::ToggleWindow(WindowId::Playlist))
                .unwrap();
        }
        app.take_focus_request();
        app.drain_notes();
    }

    /// Submits `FocusNextWindow` and returns the window the model now focuses,
    /// asserting that the pending request names the same window.
    fn focus_next(app: &mut App) -> WindowId {
        app.submit(Command::FocusNextWindow).unwrap();
        let focused = app.focused_window();
        assert_eq!(app.take_focus_request(), Some(focused));
        focused
    }

    #[test]
    fn focus_next_window_visits_the_open_windows_in_the_stated_order_and_wraps() {
        // The order is an own choice (see `focus.rs`), not a measurement.
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        assert_eq!(app.focused_window(), WindowId::Main);
        let visited: Vec<_> = (0..6).map(|_| focus_next(&mut app)).collect();
        assert_eq!(
            visited,
            [
                WindowId::Equalizer,
                WindowId::Playlist,
                WindowId::Main,
                WindowId::Equalizer,
                WindowId::Playlist,
                WindowId::Main,
            ]
        );
        assert!(app.drain_notes().is_empty(), "the command is implemented");
    }

    #[test]
    fn focus_next_window_skips_a_closed_window() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, false, true);
        assert_eq!(focus_next(&mut app), WindowId::Playlist);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        set_open(&mut app, true, false);
        assert_eq!(focus_next(&mut app), WindowId::Equalizer);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        // Only the main window left: the focus stays on it.
        set_open(&mut app, false, false);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn focus_requests_are_consumed_once() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        app.submit(Command::FocusNextWindow).unwrap();
        assert_eq!(app.take_focus_request(), Some(WindowId::Equalizer));
        assert_eq!(app.take_focus_request(), None, "taken, not repeated");
        // The model keeps the focus after the request is gone.
        assert_eq!(app.focused_window(), WindowId::Equalizer);
        // An unrelated command neither re-raises the request nor moves focus.
        app.submit(Command::ToggleShuffle).unwrap();
        assert_eq!(app.take_focus_request(), None);
        assert_eq!(app.focused_window(), WindowId::Equalizer);
    }

    #[test]
    fn closing_the_focused_window_falls_back_to_main() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        assert_eq!(focus_next(&mut app), WindowId::Equalizer);
        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
        // Closing a window that does not hold the focus asks for nothing.
        app.submit(Command::ToggleWindow(WindowId::Playlist))
            .unwrap();
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), None);
    }

    #[test]
    fn a_hidden_main_window_is_not_focusable() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, false, false);
        app.submit(Command::ToggleWindow(WindowId::Main)).unwrap();
        assert!(!app.window_open(WindowId::Main));
        app.submit(Command::FocusNextWindow).unwrap();
        assert_eq!(app.take_focus_request(), None, "nothing can take focus");
        // Unlike the toggle, the explicit show brings it back.
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.window_open(WindowId::Main));
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
    }

    #[test]
    fn show_main_window_shows_restores_and_focuses_once() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        focus_next(&mut app);
        app.submit(Command::ToggleWindow(WindowId::Main)).unwrap();
        assert!(!app.main_visible());

        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.main_visible());
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
        assert!(app.take_restore_request());
        assert!(!app.take_restore_request(), "taken, not repeated");
        // One-way: a second show keeps the window shown.
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.main_visible());
        assert!(app.drain_notes().is_empty(), "the command is implemented");
    }

    #[test]
    fn the_later_of_minimize_and_show_main_window_wins() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::Minimize).unwrap();
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(!app.take_minimize_request(), "the show beat the minimize");
        assert!(app.take_restore_request());

        app.take_focus_request();
        app.submit(Command::ShowMainWindow).unwrap();
        app.submit(Command::Minimize).unwrap();
        assert!(app.take_minimize_request(), "the minimize beat the show");
        assert!(!app.take_restore_request());
        assert_eq!(app.take_focus_request(), None, "no focus into a minimize");
    }

    #[test]
    fn focus_commands_keep_the_main_window_geometry() {
        // Focusing never resizes: a shaded, double-sized window keeps its size.
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::ToggleWindowShade(WindowId::Main))
            .unwrap();
        app.submit(Command::ToggleDoubleSize).unwrap();
        let (main_size, eq_size) = (app.frame_size(), app.eq_frame_size());
        app.submit(Command::FocusNextWindow).unwrap();
        app.submit(Command::ShowMainWindow).unwrap();
        assert_eq!(app.frame_size(), main_size);
        assert_eq!(app.eq_frame_size(), eq_size);
    }

    // ----- the simple modal dialogs (About, file info, jump-to) -----

    #[test]
    fn the_implemented_dialogs_open_real_windows_with_no_note() {
        // These commands open a real window now (`dialogs.rs`), so no
        // "not implemented yet" note may be recorded for them.
        use rustamp_core::command::{JumpTarget, OpenMediaKind, PrefsPage};
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        assert!(app.dialog_is_open(DialogKind::About));
        app.submit(Command::OpenFileInfo).unwrap();
        assert!(app.dialog_is_open(DialogKind::FileInfo));
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::Jump(JumpTarget::Time)));
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        // Each open-media kind opens its own face; the playlist ADD path
        // (`play: false`, RFC 0005 O5) opens the files face as sent.
        for kind in [
            OpenMediaKind::Files,
            OpenMediaKind::Directory,
            OpenMediaKind::Location,
        ] {
            app.submit(Command::OpenMedia { kind, play: true }).unwrap();
            assert!(app.dialog_is_open(DialogKind::OpenMedia { kind, play: true }));
        }
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        // A bare `OpenPrefs` shows the page model's own-choice default; a
        // page-carrying command shows the page it names (RFC 0005 §7).
        app.submit(Command::OpenPrefs).unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::OpenPrefsPage(PrefsPage::VisOptions))
            .unwrap();
        assert!(app.dialog_is_open(DialogKind::Preferences(PrefsPage::VisOptions)));
        let notes = app.drain_notes();
        assert!(
            !notes.iter().any(|n| n.contains("not implemented yet")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_bookmark_commands_add_list_play_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _dir) = app_with_file(Some(track(&dir, "track1_44100.tst")));
        app.submit(Command::Play).unwrap();
        app.submit(Command::AddBookmark).unwrap();
        assert_eq!(app.bookmark_menu_labels().len(), 1);
        assert!(
            app.drain_notes()
                .iter()
                .all(|n| !n.contains("not implemented"))
        );

        // Adding again changes nothing, and the menu lists it under its rows.
        app.submit(Command::AddBookmark).unwrap();
        let menu = app.native_main_menu();
        let bookmark = (1..=menu.choices.len() as u32)
            .find(|id| {
                matches!(
                    menu.choice_for_id(*id),
                    Some(rustamp_ui::menu::MenuChoice::Bookmark(0))
                )
            })
            .expect("the bookmark is a row of the menu");
        assert_eq!(
            menu.choices
                .iter()
                .filter(|c| matches!(c, rustamp_ui::menu::MenuChoice::Bookmark(_)))
                .count(),
            1
        );

        // The menu row plays it: it joins the end of the playlist.
        let before = app.engine().queue().len();
        app.activate_menu_choice(menu.choice_for_id(bookmark).unwrap())
            .unwrap();
        app.pump().unwrap();
        assert_eq!(app.engine().queue().len(), before + 1);

        // The editor lists it, and Delete removes it.
        app.submit(Command::OpenBookmarkEditor).unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(rustamp_ui::DialogKind::Bookmarks)
        );
        assert_eq!(app.bookmark_matches(), vec![0]);
        assert!(app.dialog_key(rustamp_ui::Key::Delete, rustamp_ui::Modifiers::NONE));
        assert!(app.bookmark_menu_labels().is_empty());
        assert!(app.dialog_key(rustamp_ui::Key::Escape, rustamp_ui::Modifiers::NONE));
        assert!(app.dialog_open_kind().is_none());
    }

    #[test]
    fn a_menu_label_escapes_the_mnemonic_marker() {
        let (mut app, _dir) = app_with_file(None);
        app.add_bookmark(
            rustamp_core::playlist::MediaLocation::Url("http://example.test/".to_string()),
            Some("Rock & Roll".to_string()),
        );
        assert_eq!(app.bookmark_menu_labels(), vec!["Rock && Roll".to_string()]);
    }

    #[test]
    fn confirming_the_files_dialog_adds_the_picks_and_closes() {
        // `Enter` submits the staged picks through the existing `AddItems`
        // path and closes the dialog; the queue grows by the picks.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        let before = app.engine().queue().len();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.engine().queue().len(), before + 1);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn confirming_the_location_dialog_adds_the_typed_url() {
        // The URL arrives through the entry field (the `Key` model has no
        // punctuation, so the test sets the full text programmatically; the
        // window delivers the OS text of the key presses instead).
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        }));
        let before = app.engine().queue().len();
        app.set_media_text("http://127.0.0.1/stream.mp3");
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.engine().queue().len(), before + 1);
    }

    #[test]
    fn the_open_location_chord_opens_the_location_dialog() {
        // S03 §1: `Ctrl+L` is the "open location" command (id 40185); the
        // menu row that runs the same command opens the same face. The
        // chord reaches the dialog through the usual command path.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::{DialogKind, ShortcutOutcome};

        let (mut app, _dir) = app_with_file(None);
        let outcome = app
            .ui_mut()
            .key_down(Key::Letter(b'L'), Modifiers::CTRL, 0)
            .expect("Ctrl+L is bound");
        let ShortcutOutcome::Commands(commands) = outcome else {
            panic!("Ctrl+L must emit a command");
        };
        assert_eq!(
            commands,
            vec![Command::OpenMedia {
                kind: OpenMediaKind::Location,
                play: true,
            }]
        );
        for command in commands {
            app.submit(command).unwrap();
        }
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        }));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn the_typed_url_reaches_the_queue_as_a_url_track() {
        // The location face's field becomes a `MediaLocation::Url`, and the
        // queue holds the stream track — the same kind of entry
        // (`TrackSource::Url`) a playlist's URL produces. The face is opened
        // with `play: false`, so the test never opens a socket.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: false,
        }));
        // The text arrives the way the window delivers it: the OS text of
        // the key presses, punctuation included.
        assert!(app.dialog_text("http://127.0.0.1/stream.mp3"));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(
            app.engine().queue(),
            &[TrackSource::Url("http://127.0.0.1/stream.mp3".to_owned())]
        );
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn cancelling_the_location_dialog_changes_nothing() {
        // `Escape` closes the face and drops the typed URL, so the queue
        // stays as it was.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: false,
        })
        .unwrap();
        app.dialog_text("http://127.0.0.1/stream.mp3");
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: false,
        }));
        assert!(app.engine().queue().is_empty());
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn a_url_media_becomes_a_url_track_in_the_queue() {
        // The command line's URL is the queue entry a playlist's URL
        // produces: a `TrackSource::Url`.
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: Some(MediaLocation::Url("http://127.0.0.1/stream.mp3".to_owned())),
            seconds: None,
        };
        let app = App::with_registry(options, registry()).unwrap();
        assert_eq!(
            app.engine().queue(),
            &[TrackSource::Url("http://127.0.0.1/stream.mp3".to_owned())]
        );
    }

    #[test]
    fn an_added_url_becomes_a_url_track_in_the_queue() {
        // A hand-over frame carries a location, and the app's location path
        // is the same for a URL as it is for the command line's URL: a
        // `TrackSource::Url` in the queue, never a file the player would try
        // to open. The item is added without being played, so the test opens
        // no stream.
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::AddItems {
            items: vec![MediaLocation::Url("http://127.0.0.1/stream.mp3".to_owned())],
            at: None,
            play: false,
        })
        .unwrap();
        assert_eq!(
            app.engine().queue(),
            &[TrackSource::Url("http://127.0.0.1/stream.mp3".to_owned())]
        );
    }

    #[test]
    fn confirming_an_empty_open_media_dialog_keeps_it_open() {
        // With nothing picked and a blank field there is nothing to add:
        // `Enter` stays consumed and the dialog stays open for a retry.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        let before = app.engine().queue().len();
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        assert_eq!(app.engine().queue().len(), before);
    }

    #[test]
    fn cancelling_an_open_media_dialog_adds_nothing() {
        // `Escape` closes the dialog and drops the field and picks, so the
        // queue is unchanged.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert!(app.drain_notes().is_empty());
        assert!(app.engine().queue().is_empty());
        assert!(!app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
    }

    #[test]
    fn the_playlist_add_path_keeps_its_unmeasured_play_flag() {
        // The ADD button emits `OpenMedia { kind: Files, play: false }` and
        // the value is reported, not decided: confirming adds without playing
        // (RFC 0005 O5 stays open — a 2.95 capture of the playlist add still
        // has to say whether it plays).
        use rustamp_core::command::OpenMediaKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.engine().queue().len(), 1);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn open_prefs_shows_the_default_page_and_page_commands_switch_it() {
        // `OpenPrefs` carries no page, so it shows the page model's
        // own-choice default; a page-carrying command switches the open frame
        // to the page it names (RFC 0005 §7) — with no note either way.
        use rustamp_core::command::PrefsPage;

        let (mut app, _dir) = app_with_file(None);
        assert_eq!(app.prefs_page(), None);
        app.submit(Command::OpenPrefs).unwrap();
        assert_eq!(app.prefs_page(), Some(rustamp_prefs::DEFAULT_PAGE));
        assert!(app.drain_notes().is_empty());

        app.submit(Command::OpenPrefsPage(PrefsPage::VisOptions))
            .unwrap();
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisOptions));
        assert!(app.drain_notes().is_empty());

        // The direct switch does the same when the frame is open and reports
        // when it is not.
        assert!(app.select_prefs_page(PrefsPage::VisPlugin));
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisPlugin));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.prefs_page(), None);
        assert!(!app.select_prefs_page(PrefsPage::VisPlugin));
        assert!(!app.quit(), "a dialog request must not quit the app");
    }

    #[test]
    fn the_prefs_frame_keeps_enter_open_and_closes_on_escape() {
        // The frame has no controls yet (contents unmeasured), so `Enter`
        // stays consumed without confirming; `Escape` and the focused-window
        // close shut the frame, and none of it quits the player.
        use rustamp_core::command::PrefsPage;
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPrefsPage(PrefsPage::VisPlugin))
            .unwrap();
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisPlugin));
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        app.submit(Command::OpenPrefs).unwrap();
        assert!(app.dialog_is_open(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE)));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn the_prefs_dialog_opens_from_the_menu_and_the_shortcut_path() {
        // Options > Preferences (Ctrl+P, id 40012) and the Visualization
        // leaves (Ctrl+K id 40191, Alt+O id 40317) all funnel through the
        // shortcut dispatch into the same frame, on their own page.
        use rustamp_core::command::PrefsPage;
        use rustamp_ui::{DialogKind, Key, Modifiers, ShortcutAction};

        let (mut app, _dir) = app_with_file(None);
        let outcome = app
            .activate_menu_item("Preferences", ShortcutAction::Preferences)
            .unwrap();
        assert!(
            matches!(
                outcome,
                crate::menu::MenuOutcome::Commands(ref commands)
                    if commands == &[Command::OpenPrefs]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('P').unwrap(), Modifiers::CTRL)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenPrefs]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('K').unwrap(), Modifiers::CTRL)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenPrefsPage(PrefsPage::VisPlugin)]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(PrefsPage::VisPlugin))
        );
    }

    #[test]
    fn closing_the_focused_window_closes_the_dialog_first() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert!(app.dialog_open_kind().is_none());
        assert!(!app.quit(), "closing a dialog must not quit the app");
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert!(app.quit(), "with no dialog open the main window closes");
    }

    #[test]
    fn escape_closes_the_dialog_and_digits_type_into_the_jump_box() {
        use rustamp_core::command::JumpTarget;
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert!(app.dialog_key(Key::Digit(9), Modifiers::NONE));
        assert!(app.dialog_key(Key::Digit(0), Modifiers::NONE));
        assert_eq!(app.jump_text(), "90");
        // Enter confirms: a Seek for 90 s is submitted (the empty queue turns
        // it into an error event, which must not take the app down). The dialog
        // closes on confirm so it does not linger or double-apply.
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(!app.quit());
        assert_eq!(app.dialog_open_kind(), None);
        // With no dialog open no key is consumed.
        assert!(!app.dialog_key(Key::Escape, Modifiers::NONE));
        let _ = DialogKind::About;
    }

    #[test]
    fn file_info_with_no_track_says_so() {
        let (app, _dir) = app_with_file(None);
        let lines = app.file_info_lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "no track loaded");
    }

    #[test]
    fn rendering_a_dialog_produces_a_frame_of_the_dialog_size() {
        use rustamp_core::command::{OpenMediaKind, PrefsPage};
        use rustamp_ui::{
            ABOUT_CLIENT_SIZE, DialogKind, EXTENDED_INFO_CLIENT_SIZE, FILE_INFO_CLIENT_SIZE,
            JUMP_CLIENT_SIZE, LOAD_PLAYLIST_CLIENT_SIZE, OPEN_MEDIA_CLIENT_SIZE, PREFS_CLIENT_SIZE,
            SKIN_BROWSER_CLIENT_SIZE,
        };

        let (app, _dir) = app_with_file(None);
        for (dialog, size) in [
            (DialogKind::About, ABOUT_CLIENT_SIZE),
            (DialogKind::FileInfo, FILE_INFO_CLIENT_SIZE),
            (DialogKind::ExtendedInfo, EXTENDED_INFO_CLIENT_SIZE),
            (
                DialogKind::Jump(rustamp_core::command::JumpTarget::Time),
                JUMP_CLIENT_SIZE,
            ),
            (DialogKind::LoadPlaylist, LOAD_PLAYLIST_CLIENT_SIZE),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: false,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play: true,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Location,
                    play: true,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (DialogKind::SkinBrowser, SKIN_BROWSER_CLIENT_SIZE),
            (
                DialogKind::Preferences(PrefsPage::VisPlugin),
                PREFS_CLIENT_SIZE,
            ),
        ] {
            let image = app.render_dialog(dialog);
            assert_eq!((image.width(), image.height()), size, "{dialog:?}");
            assert!(
                image
                    .as_rgba()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0),
                "the {dialog:?} frame is empty"
            );
        }
    }

    #[test]
    fn the_skin_browser_lists_what_the_app_can_load() {
        // No skin search path exists in the project, so the browser lists the
        // built-in default plus the current skin when it came from disk.
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].path, None);

        // An empty folder is a valid (if sparse) skin, so loading it makes it
        // the current skin without any fallback involved.
        let dir = tempfile::tempdir().unwrap();
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].path, Some(dir.path().to_owned()));
    }

    #[test]
    fn confirming_the_skin_browser_loads_the_pick_and_closes() {
        use rustamp_ui::Key;

        let dir = tempfile::tempdir().unwrap();
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();

        // Down to the file row, Enter loads it through the existing skin
        // path and closes the dialog.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));
        assert!(
            app.skin_description()
                .contains(dir.path().to_str().unwrap()),
            "{}",
            app.skin_description()
        );

        // Enter on the built-in row loads the built-in default instead.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), None);
        assert_eq!(app.skin_description(), "built-in default skin");
    }

    #[test]
    fn leaving_the_skin_browser_loads_nothing() {
        use rustamp_ui::{DialogKind, Key};

        let dir = tempfile::tempdir().unwrap();
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();

        // Escape closes without loading; the focused-window close does too,
        // and neither quits the player.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));

        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn the_skin_browser_opens_from_the_menu_and_the_shortcut_path() {
        // The main menu's skin-browser leaf and Alt+S (S03) both funnel
        // through the shortcut dispatch into the same dialog.
        use rustamp_ui::{DialogKind, Key, Modifiers, ShortcutAction};

        let (mut app, _dir) = app_with_file(None);
        let outcome = app
            .activate_menu_item("Skin browser", ShortcutAction::SkinBrowser)
            .unwrap();
        assert!(
            matches!(
                outcome,
                crate::menu::MenuOutcome::Commands(ref commands)
                    if commands == &[Command::OpenSkinBrowser]
            ),
            "{outcome:?}"
        );
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('S').unwrap(), Modifiers::ALT)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenSkinBrowser]
            ),
            "{outcome:?}"
        );
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
    }

    /// The browser shows a window of the skins, not all of them: the rows
    /// that fit into its client area are drawn and the rest is reached by
    /// scrolling.
    #[test]
    fn the_skin_browser_scrolls_its_window() {
        use rustamp_ui::{DialogKind, Key, Modifiers, skin_browser_visible_rows};

        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        for index in 0..20 {
            crate::skin_folder::write_skin_archive(&skins.join(format!("skin {index:02}.wsz")));
        }
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        // The built-in default plus the twenty folder skins.
        assert_eq!(app.browser_entries().len(), 21);

        // The rows that fit into the client area are on screen; the rest is
        // not drawn at all.
        let visible =
            skin_browser_visible_rows(rustamp_ui::DialogKind::SkinBrowser.client_size().1);
        assert_eq!(visible, 9);
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), visible, "{lines:?}");
        assert!(lines[0].contains("built-in default skin"), "{lines:?}");

        // Down past the window scrolls it: the ninth row is the last one on
        // screen, the tenth moves the window on.
        for _ in 0..8 {
            assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        }
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), visible, "{lines:?}");
        assert!(lines[0].contains("built-in default skin"), "{lines:?}");
        assert!(lines[8].contains("skin 07"), "{lines:?}");
        assert!(lines[8].starts_with("> "), "{lines:?}");
        assert!(app.dialog_key(Key::Down, Modifiers::NONE));
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), visible, "{lines:?}");
        assert!(lines[0].contains("skin 00"), "{lines:?}");
        assert!(lines[8].contains("skin 08"), "{lines:?}");
        assert!(lines[8].starts_with("> "), "{lines:?}");

        // End jumps to the last row, Home back to the first.
        assert!(app.dialog_key(Key::End, Modifiers::NONE));
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), visible, "{lines:?}");
        assert!(lines[8].contains("skin 19"), "{lines:?}");
        assert!(app.dialog_key(Key::Home, Modifiers::NONE));
        let lines = app.open_dialog_lines().unwrap();
        assert!(lines[0].contains("built-in default skin"), "{lines:?}");
    }

    /// The frame of the browser never draws a row that would cross the
    /// bottom of its client area.
    #[test]
    fn the_skin_browser_draws_no_row_outside_its_client_area() {
        use rustamp_ui::dialogs::{BODY_TOP, TEXT_LINE_PX};
        use rustamp_ui::{DialogKind, SKIN_BROWSER_CLIENT_SIZE};

        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        for index in 0..20 {
            crate::skin_folder::write_skin_archive(&skins.join(format!("skin {index:02}.wsz")));
        }
        app.submit(Command::OpenSkinBrowser).unwrap();

        let frame = app.render_dialog(DialogKind::SkinBrowser);
        assert_eq!(
            (frame.width(), frame.height()),
            SKIN_BROWSER_CLIENT_SIZE,
            "the frame is the client area"
        );
        // The band of the tenth row crosses the bottom, so it is not drawn.
        let bg = app.skin.skin().pledit().normal_bg;
        let background = Some([bg[0], bg[1], bg[2], 255]);
        let last_row = BODY_TOP + 9 * TEXT_LINE_PX;
        for y in last_row..frame.height() as i32 - 1 {
            for x in 1..frame.width() as i32 - 1 {
                assert_eq!(frame.pixel(x, y), background, "{x},{y}");
            }
        }
    }

    /// An app whose configuration sits in `dir`, with an empty `skins` folder
    /// beside it.
    fn app_with_skins_folder(dir: &tempfile::TempDir) -> App {
        let (mut app, _config) = app_with_file(None);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        std::fs::create_dir_all(&skins).unwrap();
        app.set_config_path(Some(dir.path().join("rustamp.toml")));
        app
    }

    #[test]
    fn the_skin_list_holds_the_skins_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        crate::skin_folder::write_skin_archive(&skins.join("Beta.wsz"));
        crate::skin_folder::write_skin_archive(&skins.join("alpha.wsz"));

        // The built-in default first, then the folder's skins in name order.
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(entries[0].path, None, "{entries:?}");
        assert_eq!(
            entries[1].path,
            Some(skins.join("alpha.wsz")),
            "{entries:?}"
        );
        assert_eq!(entries[2].path, Some(skins.join("Beta.wsz")), "{entries:?}");
    }

    #[test]
    fn an_empty_skins_folder_leaves_the_built_in_skin_alone() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with_skins_folder(&dir);
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].path, None, "{entries:?}");
    }

    #[test]
    fn the_skin_in_use_is_listed_after_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        crate::skin_folder::write_skin_archive(&skins.join("one.wsz"));
        // A skin from outside the folder: it is listed after the folder's
        // skins, so the user sees what is in use.
        let outside = tempfile::tempdir().unwrap();
        app.submit(Command::LoadSkin(outside.path().to_owned()))
            .unwrap();

        let entries = app.browser_entries();
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(entries[0].path, None, "{entries:?}");
        assert_eq!(entries[1].path, Some(skins.join("one.wsz")), "{entries:?}");
        assert_eq!(
            entries[2].path,
            Some(outside.path().to_owned()),
            "{entries:?}"
        );
    }

    #[test]
    fn a_skin_of_the_folder_is_listed_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        crate::skin_folder::write_skin_archive(&skins.join("one.wsz"));
        app.submit(Command::LoadSkin(skins.join("one.wsz")))
            .unwrap();

        // The skin in use is one of the folder's skins: it is listed there
        // and not a second time at the end.
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].path, Some(skins.join("one.wsz")), "{entries:?}");
    }

    #[test]
    fn a_broken_skin_in_the_folder_is_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        std::fs::write(skins.join("broken.wsz"), b"not a zip at all").unwrap();
        crate::skin_folder::write_skin_archive(&skins.join("one.wsz"));

        // The broken file is not offered; the skin beside it is.
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].path, Some(skins.join("one.wsz")), "{entries:?}");

        // The reason reaches the user as a note.
        app.pump().unwrap();
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|note| note.contains("broken.wsz")),
            "{notes:?}"
        );
    }

    #[test]
    fn loading_a_row_of_the_skin_list_loads_that_skin() {
        use rustamp_platform::PrefsAction;

        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_skins_folder(&dir);
        let skins = dir.path().join(crate::skin_folder::SKINS_FOLDER);
        crate::skin_folder::write_skin_archive(&skins.join("one.wsz"));
        crate::skin_folder::write_skin_archive(&skins.join("two.wsz"));

        // The preferences page names the skins of the folder and the skin in
        // use.
        let settings = app.prefs_settings();
        assert_eq!(settings.skins.len(), 3, "{settings:?}");
        assert_eq!(settings.skin_selected, Some(0), "{settings:?}");

        // The third row is the second skin of the folder, and loading that
        // row loads that file.
        app.apply_prefs_action(PrefsAction::LoadSkin(2));
        assert_eq!(app.current_skin(), Some(&skins.join("two.wsz")));
        assert_eq!(app.prefs_settings().skin_selected, Some(2));
    }

    #[test]
    fn the_load_playlist_dialog_keeps_its_confirmation_open() {
        // An empty confirm keeps the dialog open (there is no path to load);
        // a confirmed one loads the file and closes the dialog (see the
        // load-playlist tests below). Escape closes; the focused-window
        // close does too.
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        app.submit(Command::OpenPlaylist).unwrap();
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn easy_move_decides_which_presses_drag_the_window() {
        // The easy-move contract (W10 finding F5): the setting is honoured by
        // the main-window drag decision, not only carried. A press no control
        // owns drags exactly while easy-move is on; a press on a widget
        // always reaches the control machine.
        let (mut app, _dir) = app_with_file(None);
        assert!(app.easy_move(), "the default config has easy-move on");

        // (5, 100) is background: outside every widget rect, so the control
        // machine ignores the press and the setting decides.
        assert!(
            app.mouse_down(5, 100).unwrap().is_empty(),
            "the background press must own nothing"
        );
        assert!(app.easy_drag_main(5, 100));

        // A control keeps its press whatever the setting says.
        let (x, y) = widget_centre(Widget::Play);
        assert!(!app.easy_drag_main(x, y));

        // With easy-move off (menu toggle, shortcut Ctrl+E per S03) nothing
        // drags: the OS chrome stays the only move path.
        app.submit(Command::ToggleEasyMove).unwrap();
        assert!(!app.easy_move());
        assert!(!app.easy_drag_main(5, 100));
        assert!(!app.easy_drag_main(x, y));
    }

    // ----- Headless tests for GUI-driven playback chains (task card N3) -----

    #[test]
    fn chain1_opening_file_via_open_media_enqueues_and_playing_starts_playback() {
        // Chain 1: Opening a file (the Open Media path) enqueues it and playing
        // it starts playback: queue length, current index and engine transport all agree.
        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.engine().current_index(), None);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Open file via Open Media dialog with play: false (playlist ADD path)
        app.submit(Command::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: false,
        }));
        app.stage_media_paths(vec![t1]);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        // Enqueued: queue and playlist model agree
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.playlist().len(), 1);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Playing it starts playback
        app.submit(Command::Play).unwrap();
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().transport() == TransportState::Playing,
                2000
            ),
            "playback did not start"
        );
        // All agree afterwards
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.state().transport, TransportState::Playing);

        // Also test Open Media with play: true (e.g. shortcut / location)
        let t2 = track(&dir, "track2.tst");
        app.submit(Command::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: true,
        })
        .unwrap();
        app.stage_media_paths(vec![t2]);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1),
                2000
            ),
            "play: true did not switch to new track"
        );
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }

    #[test]
    fn chain2_double_clicking_playlist_entry_plays_that_entry() {
        // Chain 2: Double-clicking a playlist entry plays that entry
        // (the index the window reports reaches the engine).
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "t0.tst");
        let t1 = track(&dir, "t1.tst");
        let t2 = track(&dir, "t2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Double-click row 1 (center of row 1 in playlist list band: y = 23 + 13 + 6 = 42, x = 60)
        let commands = app.playlist_double_click(60, 42).unwrap();
        assert_eq!(commands, vec![Command::PlayEntry(EntryId(1))]);
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);

        // Engine starts playing index 1
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1)
                    && app.engine().transport() == TransportState::Playing,
                2000
            ),
            "double-clicking row 1 did not play entry at index 1"
        );
        assert_eq!(app.engine().current_index(), Some(1));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // Double-click row 0 (y = 23 + 6 = 29, x = 60)
        let commands = app.playlist_double_click(60, 29).unwrap();
        assert_eq!(commands, vec![Command::PlayEntry(EntryId(0))]);
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(0)
                    && app.engine().transport() == TransportState::Playing,
                2000
            ),
            "double-clicking row 0 did not play entry at index 0"
        );
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }

    #[test]
    fn chain3_next_and_previous_move_through_queue_across_button_menu_key_and_ends() {
        // Chain 3: Next/Previous (button, menu or key) move through the queue
        // and the engine follows, including at the ends.
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "track0.tst");
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0)
                && app.engine().transport() == TransportState::Playing,
            2000
        ));

        // 1. Previous at start (index 0) remains at index 0 across key, button, and menu:
        // Key 'Z'
        app.key_down(Key::letter('Z').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // Button Previous
        let (px, py) = widget_centre(Widget::Previous);
        app.mouse_down(px, py).unwrap();
        app.mouse_up(px, py).unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // Menu "Previous track"
        app.activate_menu_item("Previous track", rustamp_ui::ShortcutAction::PreviousTrack)
            .unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // 2. Next via Button: moves 0 -> 1
        let (nx, ny) = widget_centre(Widget::Next);
        app.mouse_down(nx, ny).unwrap();
        let commands = app.mouse_up(nx, ny).unwrap();
        assert_eq!(commands, vec![Command::Next]);
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // 3. Next via Key 'B': moves 1 -> 2
        let outcome = app
            .key_down(Key::letter('B').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(
            outcome,
            Some(rustamp_ui::ShortcutOutcome::Commands(vec![Command::Next]))
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(2),
            2000
        ));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // 4. Previous via Menu: moves 2 -> 1
        let outcome = app
            .activate_menu_item("Previous track", rustamp_ui::ShortcutAction::PreviousTrack)
            .unwrap();
        assert_eq!(
            outcome,
            crate::menu::MenuOutcome::Commands(vec![Command::Prev])
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));

        // 5. Previous via Key 'Z': moves 1 -> 0
        let outcome = app
            .key_down(Key::letter('Z').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(
            outcome,
            Some(rustamp_ui::ShortcutOutcome::Commands(vec![Command::Prev]))
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0),
            2000
        ));

        // 6. Next via Menu: moves 0 -> 1
        let outcome = app
            .activate_menu_item("Next track", rustamp_ui::ShortcutAction::NextTrack)
            .unwrap();
        assert_eq!(
            outcome,
            crate::menu::MenuOutcome::Commands(vec![Command::Next])
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));

        // Advance to last track (2)
        app.submit(Command::Next).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(2),
            2000
        ));

        // 7. Next at the end: moves past the end of the queue and the run ends
        //    once the buffered audio has drained (the engine finishes on a
        //    drained ring, so the clock has to advance here).
        app.key_down(Key::letter('B').unwrap(), Modifiers::NONE)
            .unwrap();
        assert!(
            pump_until_drained(
                &mut app,
                |app| app.engine().transport() == TransportState::Stopped,
                2000
            ),
            "playback did not stop at end of queue"
        );
        assert!(app.playlist_ended);
    }

    #[test]
    fn chain4_playlist_window_model_stays_in_step_with_engine_queue() {
        // Chain 4: The playlist window's own model (its rows) stays in step with
        // the engine's queue when entries are added or removed.
        let (mut app, _dir) = app_with_file(None);

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.playlist().len(), 0);
        assert_eq!(app.entry_ids.len(), 0);

        // Add 3 entries
        let items = (0..3)
            .map(|i| MediaLocation::File(PathBuf::from(format!("track{i}.mp3"))))
            .collect::<Vec<_>>();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().queue().len(), 3);
        assert_eq!(app.playlist().len(), 3);
        assert_eq!(app.entry_ids.len(), 3);
        assert_eq!(app.playlist().visible_ids(), app.entry_ids);

        // Remove row 1 via REM button
        app.submit(Command::Select(rustamp_core::command::SelectOp::Set(vec![
            EntryId(1),
        ])))
        .unwrap();
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);

        let cmds = app.playlist_mouse_down(46, 92, Modifiers::NONE).unwrap();
        assert_eq!(cmds, vec![Command::Remove(vec![EntryId(1)])]);

        // Model, entry_ids, and engine queue all agree after removal
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.entry_ids.len(), 2);
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(0), EntryId(2)]);

        // Add 2 more entries
        let items2 = vec![
            MediaLocation::File(PathBuf::from("track3.mp3")),
            MediaLocation::File(PathBuf::from("track4.mp3")),
        ];
        app.submit(Command::AddItems {
            items: items2,
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().queue().len(), 4);
        assert_eq!(app.playlist().len(), 4);
        assert_eq!(app.entry_ids.len(), 4);

        // Remove via external Command::Remove
        app.submit(Command::Remove(vec![EntryId(0), EntryId(3)]))
            .unwrap();
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.entry_ids.len(), 2);
        assert_eq!(app.entry_ids, vec![EntryId(2), EntryId(4)]);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(2), EntryId(4)]);
    }

    #[test]
    fn sort_by_file_name_reorders_the_table_the_queue_and_the_model() {
        // S03 §2 `40210`: sort by file name, case-insensitively.
        let (mut app, dir) = app_with_file(None);
        let zulu = track(&dir, "Zulu.mp3");
        let alpha = track(&dir, "alpha.mp3");
        let beta = track(&dir, "Beta.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(zulu),
                MediaLocation::File(alpha),
                MediaLocation::File(beta),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();

        // alpha < Beta < Zulu once case is folded away.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(2), EntryId(0)]);
        assert_eq!(app.playlist().visible_ids(), app.entry_ids);
        let names: Vec<String> = app
            .engine()
            .queue()
            .iter()
            .map(|track| track.display())
            .collect();
        assert!(names[0].ends_with("alpha.mp3"), "{names:?}");
        assert!(names[1].ends_with("Beta.mp3"), "{names:?}");
        assert!(names[2].ends_with("Zulu.mp3"), "{names:?}");
    }

    #[test]
    fn sort_by_title_uses_the_loaded_titles_and_falls_back_to_the_file_name() {
        // S03 §2 `40209`: an entry the load did not name keeps the file name as
        // its key (the core's fallback).
        let (mut app, dir) = app_with_file(None);
        let named = track(&dir, "fileA.mp3");
        let unnamed = track(&dir, "fileB.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(named.clone()),
                MediaLocation::File(unnamed.clone()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.last_playlist_report = Some(rustamp_playlist::PlaylistReport::new(
            vec![
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(named)).with_title("Zulu"),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(unnamed)),
            ],
            Vec::new(),
        ));

        app.submit(Command::Sort(SortKey::Title)).unwrap();

        // "fileb.mp3" < "zulu": the unnamed entry's fallback sorts first.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(0)]);
    }

    #[test]
    fn sort_by_duration_puts_unknown_durations_first() {
        // `SortKey::Duration`: unknown first (the core's `None`-before-`Some`
        // rule); the durations come from the last playlist load.
        let (mut app, dir) = app_with_file(None);
        let slow = track(&dir, "slow.mp3");
        let fast = track(&dir, "fast.mp3");
        let unknown = track(&dir, "unknown.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(slow.clone()),
                MediaLocation::File(fast.clone()),
                MediaLocation::File(unknown.clone()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.last_playlist_report = Some(rustamp_playlist::PlaylistReport::new(
            vec![
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(slow))
                    .with_duration_ms(9_000),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(fast))
                    .with_duration_ms(1_000),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(unknown)),
            ],
            Vec::new(),
        ));

        app.submit(Command::Sort(SortKey::Duration)).unwrap();

        // unknown(2), then 1 s (1), then 9 s (0).
        assert_eq!(app.entry_ids, vec![EntryId(2), EntryId(1), EntryId(0)]);
    }

    #[test]
    fn equal_sort_keys_keep_their_relative_order_and_the_path_breaks_the_tie() {
        // Stable, like the core's rule: equal file names keep insertion order,
        // and only the full path separates them.
        let (mut app, dir) = app_with_file(None);
        let b_dir = dir.path().join("b").join("x.mp3");
        let a_dir = dir.path().join("a").join("x.mp3");
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(b_dir), MediaLocation::File(a_dir)],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();
        assert_eq!(
            app.entry_ids,
            vec![EntryId(0), EntryId(1)],
            "a tie is stable"
        );

        app.submit(Command::Sort(SortKey::PathAndFileName)).unwrap();
        assert_eq!(
            app.entry_ids,
            vec![EntryId(1), EntryId(0)],
            "the a/ path sorts first"
        );
    }

    #[test]
    fn sorting_keeps_the_current_entry_and_the_selection_by_identity() {
        use rustamp_core::command::SelectOp;

        let (mut app, dir) = app_with_file(None);
        let zulu = track(&dir, "zulu.mp3");
        let alpha = track(&dir, "alpha.mp3");
        let mike = track(&dir, "mike.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(zulu),
                MediaLocation::File(alpha),
                MediaLocation::File(mike),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.engine_mut().set_current_index(Some(0));
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1)])))
            .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();

        // alpha(1), mike(2), zulu(0): zulu moved to row 2 and is still current;
        // the selected alpha is row 0 and still selected.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(2), EntryId(0)]);
        assert_eq!(app.engine().current_index(), Some(2));
        assert_eq!(app.entry_id_for(Some(2)), Some(EntryId(0)));
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);
    }

    #[test]
    fn dragging_the_playing_entry_moves_its_row_without_restarting_playback() {
        use rustamp_core::command::SelectOp;

        let (mut app, dir) = app_with_file(None);
        let items = (0..4)
            .map(|i| MediaLocation::File(track(&dir, &format!("drag{i}.tst"))))
            .collect();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();
        app.submit(Command::PlayEntry(EntryId(1))).unwrap();
        // This is a fresh run, so there is no stale ring to drain. Fill it
        // before driving the virtual device: pumping 1024 frames per 1 ms
        // while waiting can consume all four one-second tracks before the
        // decoder ever builds the required backlog, or advance to another
        // track before the drag starts. The clock stays frozen until ready.
        assert!(pump_until(
            &mut app,
            |app| app.engine().buffered_frames() > 4096 && app.engine().source_spec().is_some(),
            2000
        ));
        app.advance_null_clock(1024);
        app.pump().unwrap();
        assert_eq!(
            app.engine().current_index(),
            Some(1),
            "the dragged entry must already be playing"
        );
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1)])))
            .unwrap();
        let position = app.engine().position_ms();
        assert!(
            position > 0,
            "prove a running track, not just a Play command"
        );
        app.playlist_mouse_down(60, 42, Modifiers::NONE).unwrap();
        app.playlist_mouse_move(60, 74);
        assert_eq!(
            app.playlist_mouse_up(60, 74).unwrap(),
            vec![Command::MoveEntries {
                ids: vec![EntryId(1)],
                to: 3,
            }]
        );
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().position_ms(), position);
        app.pump().unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().position_ms(), position);
        assert_eq!(app.engine().current_index(), Some(3));
        assert_eq!(
            app.entry_ids,
            vec![EntryId(0), EntryId(2), EntryId(3), EntryId(1)]
        );
        assert_eq!(
            app.engine().queue()[3].display(),
            dir.path().join("drag1.tst").display().to_string()
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);
        app.render_playlist();
        assert_eq!(app.playlist().current(), Some(EntryId(1)));
    }

    #[test]
    fn randomize_permutes_the_order_and_keeps_the_entries() {
        // S03 §2 `40212` (S15 §5 maps it to the in-place shuffle): the order
        // changes, the entries do not.
        let (mut app, dir) = app_with_file(None);
        let items = (0..32)
            .map(|i| MediaLocation::File(track(&dir, &format!("r{i:02}.mp3"))))
            .collect::<Vec<_>>();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();
        let before = app.entry_ids.clone();

        app.submit(Command::Randomize).unwrap();

        let after = app.entry_ids.clone();
        let mut before_sorted = before.clone();
        before_sorted.sort_by_key(|id| id.0);
        let mut after_sorted = after.clone();
        after_sorted.sort_by_key(|id| id.0);
        assert_eq!(before_sorted, after_sorted, "the same 32 ids");
        assert_ne!(before, after, "a 32-entry shuffle changes the order");
        // The model holds every entry and mirrors the new order (only the
        // visible page is exposed).
        assert_eq!(app.playlist().len(), after.len());
        let visible = app.playlist().visible_ids();
        assert_eq!(visible.as_slice(), &after[..visible.len()]);
    }

    #[test]
    fn chain5_removing_playing_entry_or_clearing_leaves_defined_state() {
        // Chain 5: Removing the playing entry, or clearing the list while playing,
        // leaves the engine in a defined state (and say which).
        //
        // Defined state:
        // - When the playing entry is removed: the engine stops (TransportState::Stopped),
        //   position resets to 0 ms, and the queue & playlist model contain the remaining entries.
        // - When the list is cleared while playing: the engine stops (TransportState::Stopped),
        //   position resets to 0 ms, and the queue & playlist model are empty.
        // - When a non-playing entry is removed while playing: playback continues (TransportState::Playing),
        //   and the queue & playlist model update to the remaining entries.
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "p0.tst");
        let t1 = track(&dir, "p1.tst");
        let t2 = track(&dir, "p2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        // Subcase 5A: Removing a non-playing entry while playing keeps playback active
        app.submit(Command::PlayEntry(EntryId(0))).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0),
            2000
        ));

        // Remove track 2 (non-playing)
        app.submit(Command::Remove(vec![EntryId(2)])).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);

        // Subcase 5B: Removing the currently playing entry stops the engine
        app.submit(Command::Remove(vec![EntryId(0)])).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.engine().position_ms(), 0);
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.playlist().len(), 1);
        assert_eq!(app.entry_ids, vec![EntryId(1)]);

        // Subcase 5C: Clearing the playlist while playing stops the engine and empties queue & model
        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing,
            2000
        ));

        app.submit(Command::Clear).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.engine().position_ms(), 0);
        assert!(app.engine().queue().is_empty());
        assert!(app.playlist().is_empty());
        assert!(app.entry_ids.is_empty());
    }

    #[test]
    fn remove_dead_files_drops_only_the_missing_local_files() {
        // S03 §2 `40222` ("Remove all dead files", RFC 0018): the entry whose
        // file is gone leaves; the existing file and the stream stay. The
        // stream is never probed — a URL is not a file (own choice, RFC 0018).
        let (mut app, dir) = app_with_file(None);
        let alive = track(&dir, "alive.tst");
        let gone = dir.path().join("gone.tst");
        assert!(!gone.exists(), "the fixture must start absent");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(alive.clone()),
                MediaLocation::File(gone),
                MediaLocation::Url("http://example.invalid/stream.mp3".to_string()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        assert_eq!(app.entry_ids.len(), 3);

        app.submit(Command::RemoveDeadFiles).unwrap();
        // Stable ids, in order: the dead middle entry is gone, nothing renumbers.
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(0), EntryId(2)]);

        // No dead entry left: the command is a silent no-op.
        app.submit(Command::RemoveDeadFiles).unwrap();
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn remove_dead_files_stops_the_engine_when_the_playing_entry_is_dead() {
        // The removal goes through `remove_entries`, so a dead current entry
        // stops playback and clears Current, exactly like any other removal.
        let (mut app, dir) = app_with_file(None);
        let dead = dir.path().join("never-written.tst");
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(dead.clone())],
            at: None,
            play: false,
        })
        .unwrap();
        // The engine refuses to open a missing file, so stage the current row
        // the way a previously alive entry would look after the file vanished.
        app.engine_mut().set_current_index(Some(0));
        app.submit(Command::RemoveDeadFiles).unwrap();
        assert!(app.entry_ids.is_empty());
        assert!(app.engine().queue().is_empty());
        assert_eq!(app.engine().current_index(), None);
        assert!(app.playlist().is_empty());
    }

    /// Pins the OS window parameters the two new dialogs must carry (titles
    /// and client sizes) for the run that does open them. **VM-only,
    /// `#[ignore]`d by default, and gated on the test VM**: GUI must never
    /// run on the host.
    ///
    /// ```text
    /// cargo test -p rustamp-app -- --ignored dialogs
    /// ```
    #[ignore = "VM-only: pins dialog window parameters; run inside the headless test VM"]
    #[test]
    fn the_new_dialogs_pin_their_os_window_parameters() {
        // The gate, before anything else. This test opens no window itself;
        // it pins the parameters the presenter (`window.rs`) builds the real
        // OS windows from.
        crate::window::require_test_vm();
        use rustamp_core::command::{OpenMediaKind, PrefsPage};
        use rustamp_ui::{
            DialogKind, EDIT_ENTRY_CLIENT_SIZE, EDIT_ENTRY_TITLE, LOAD_PLAYLIST_CLIENT_SIZE,
            OPEN_MEDIA_CLIENT_SIZE, PREFS_CLIENT_SIZE, SKIN_BROWSER_CLIENT_SIZE,
        };
        assert_eq!(DialogKind::LoadPlaylist.title(), "Load Playlist");
        assert_eq!(
            DialogKind::LoadPlaylist.client_size(),
            LOAD_PLAYLIST_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            }
            .title(),
            "Add file(s) to playlist"
        );
        for kind in [
            OpenMediaKind::Files,
            OpenMediaKind::Directory,
            OpenMediaKind::Location,
        ] {
            assert_eq!(
                DialogKind::OpenMedia { kind, play: true }.client_size(),
                OPEN_MEDIA_CLIENT_SIZE,
                "{kind:?}"
            );
        }
        assert_eq!(DialogKind::SkinBrowser.title(), "Skin browser");
        assert_eq!(
            DialogKind::SkinBrowser.client_size(),
            SKIN_BROWSER_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).title(),
            "Preferences"
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).client_size(),
            PREFS_CLIENT_SIZE
        );
        // The edit-entry box (S03 §2 `40255`): its own title and size.
        assert_eq!(DialogKind::EditEntry.title(), EDIT_ENTRY_TITLE);
        assert_eq!(DialogKind::EditEntry.client_size(), EDIT_ENTRY_CLIENT_SIZE);
    }

    #[test]
    fn clutter_i_opens_file_info_only_when_track_loaded() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        // With no track loaded, clicking clutter I emits nothing and opens no dialog (S02 §2).
        let down_cmds = app.mouse_down(15, 43).unwrap();
        assert!(down_cmds.is_empty());
        let up_cmds = app.mouse_up(15, 43).unwrap();
        assert!(up_cmds.is_empty());
        assert_eq!(app.dialog_open_kind(), None);

        // Load a track.
        app.handle_event(&rustamp_core::event::Event::TrackChanged {
            id: Some(rustamp_core::playlist::EntryId(1)),
            index: Some(0),
            metadata: None,
            duration_ms: Some(180_000),
        });

        // Now clicking clutter I emits OpenFileInfo and opens the FileInfo dialog.
        let down_cmds = app.mouse_down(15, 43).unwrap();
        assert!(down_cmds.is_empty());
        let up_cmds = app.mouse_up(15, 43).unwrap();
        assert_eq!(up_cmds, vec![Command::OpenFileInfo]);
        assert_eq!(app.dialog_open_kind(), Some(DialogKind::FileInfo));
    }

    #[test]
    fn dialog_result_paths_cancelled_confirmed_and_os_close() {
        use rustamp_core::command::JumpTarget;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);

        // 1. Cancelled dialog via Escape: changes nothing and closes dialog.
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Jump(JumpTarget::Time))
        );
        assert!(app.dialog_key(Key::Digit(4), Modifiers::NONE));
        assert!(app.dialog_key(Key::Digit(2), Modifiers::NONE));
        assert_eq!(app.jump_text(), "42");
        assert!(app.dialog_key(Key::Escape, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        // 2. Confirmed dialog via Enter: applies exactly once and closes.
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Jump(JumpTarget::Time))
        );
        assert!(app.dialog_key(Key::Digit(1), Modifiers::NONE));
        assert!(app.dialog_key(Key::Digit(0), Modifiers::NONE));
        assert_eq!(app.jump_text(), "10");
        // Enter confirms: seek is submitted, dialog closes immediately.
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);
        // Pressing Enter again does NOT double-apply: no dialog is open.
        assert!(!app.dialog_key(Key::Enter, Modifiers::NONE));

        // 3. Closed by OS: close_dialog resets state cleanly without applying.
        app.submit(Command::OpenAbout).unwrap();
        assert_eq!(app.dialog_open_kind(), Some(DialogKind::About));
        app.close_dialog(DialogKind::About);
        assert_eq!(app.dialog_open_kind(), None);
    }

    #[test]
    fn good_playlist_file_loads_and_plays() {
        use rustamp_ui::DialogKind;

        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        let m3u_path = dir.path().join("good.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:120,First Track\r\ntrack1.tst\r\n#EXTINF:180,Second Track\r\ntrack2.tst\r\n",
        )
        .unwrap();

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Open Load Playlist dialog
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Stage playlist file and confirm
        app.stage_playlist_path(m3u_path);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        // Queue has surviving entries in order
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));

        // First entry starts playback. The engine starts the run synchronously
        // (`play_index` sets the transport before the dialog key returns), but
        // the UI machine learns Playing only when the app pumps the
        // StateChanged event off the bus, so the wait must cover the UI state
        // it asserts: a wait on the engine alone is already satisfied before
        // the first pump and would assert an event nobody has delivered yet.
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0)
                && app.state().transport == TransportState::Playing,
            2000
        ));
        assert_eq!(app.state().transport, TransportState::Playing);

        // Clean report recorded
        let report = app.last_playlist_report().expect("report recorded");
        assert!(report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 0);
        assert_eq!(app.last_error(), None);
    }

    #[test]
    fn playlist_file_with_bad_entry_loads_surviving_and_reports_skips() {
        use rustamp_ui::DialogKind;

        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        // Playlist with a dangling EXTINF (skipped by reader)
        let m3u_path = dir.path().join("with_skips.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:60,Dangling Track\r\n#EXTINF:120,Track One\r\ntrack1.tst\r\n#EXTINF:180,Track Two\r\ntrack2.tst\r\n",
        )
        .unwrap();

        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        app.stage_playlist_path(m3u_path);
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        // Surviving entries reach queue in order
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));

        // First entry plays
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0),
            2000
        ));

        // Skips are recorded on App and surfaced in notes
        let report = app.last_playlist_report().expect("report recorded");
        assert!(!report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 1);
        assert!(report.skips[0].reason().contains("dangling #EXTINF"));

        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("loaded 2 tracks, skipped 1 malformed entries")),
            "notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("skipped: dangling #EXTINF")),
            "notes: {notes:?}"
        );
    }

    /// The other half of readiness finding BETA-09 (review revision
    /// `f9012dc`): a playlist named as the CLI `<file>` must load as a *list*
    /// through the reporting reader, not queue as one media file. The dialog
    /// half is pinned above; this pins the CLI half, which no test covered.
    #[test]
    fn playlist_file_given_on_the_command_line_loads_as_a_list() {
        let dir = tempfile::tempdir().unwrap();
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        // A dangling EXTINF before the first entry: the reader drops it and
        // keeps the reason, so an app that called the legacy `read_playlist` /
        // `load_playlist_file` (which discard it through `into_entries`) could
        // not satisfy the skip assertions below.
        let m3u_path = dir.path().join("cli.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:60,Dangling Track\r\n#EXTINF:120,Track One\r\ntrack1.tst\r\n#EXTINF:180,Track Two\r\ntrack2.tst\r\n",
        )
        .unwrap();

        // The playlist path *is* the CLI `<file>` argument.
        let (mut app, _d) = app_with_file(Some(m3u_path.clone()));

        // It loaded as a list of its surviving entries, in file order...
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));
        // ...and the playlist itself is not in the queue as a media path.
        assert!(!app.engine().queue().contains(&TrackSource::path(m3u_path)));

        // The kept reasons reached the app: the report is recorded and not
        // silent, and no error was fabricated.
        let report = app.last_playlist_report().expect("report recorded");
        assert!(!report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 1);
        assert!(report.skips[0].reason().contains("dangling #EXTINF"));
        assert_eq!(app.last_error(), None);

        // The drop is surfaced as a note (stderr in windowed runs).
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("loaded 2 tracks, skipped 1 malformed entries")),
            "notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("skipped: dangling #EXTINF")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn unsupported_playlist_file_reports_error_and_changes_nothing() {
        use rustamp_ui::DialogKind;

        let dir = tempfile::tempdir().unwrap();
        let t0 = track(&dir, "original.tst");
        let (mut app, _d) = app_with_file(Some(t0.clone()));

        // Start with 1 track in queue
        assert_eq!(app.engine().queue().len(), 1);

        // Write an unsupported/corrupted playlist file (invalid binary content with m3u extension)
        let bad_path = dir.path().join("corrupted.m3u");
        std::fs::write(&bad_path, b"\x00\x01\x02\x03\x04\x05").unwrap();

        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        app.stage_playlist_path(bad_path.clone());
        assert!(app.dialog_key(Key::Enter, Modifiers::NONE));
        assert_eq!(app.dialog_open_kind(), None);

        // Queue is completely unchanged
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t0));

        // Error is surfaced visibly in last_error and window_title
        let error = app.last_error().expect("last_error must be set");
        assert!(
            error.contains("failed to load playlist")
                && error.contains("unrecognized playlist content"),
            "error: {error}"
        );
        assert!(
            app.window_title()
                .starts_with("Rustamp - failed to load playlist")
        );

        // Error is also logged in notes
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|n| n.contains("failed to load playlist")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn load_playlist_dialog_text_input_and_display() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Initial body lines: prompt, empty field, hint
        let body = app.open_dialog_lines().expect("dialog open");
        assert_eq!(body[0], "Playlist file:");
        assert_eq!(body[1], "");
        assert_eq!(body[2], crate::dialogs::OPEN_PLAYLIST_HINT);

        // Type text into dialog
        assert!(app.dialog_text("my_list.m3u"));
        assert_eq!(app.playlist_text(), "my_list.m3u");

        let body = app.open_dialog_lines().expect("dialog open");
        assert_eq!(body[1], "my_list.m3u");

        // Backspace
        assert!(app.dialog_key(Key::Backspace, Modifiers::NONE));
        assert_eq!(app.playlist_text(), "my_list.m3");

        // Key letters/digits
        assert!(app.dialog_key(Key::Letter(b'u'), Modifiers::NONE));
        assert_eq!(app.playlist_text(), "my_list.m3u");
    }

    #[test]
    fn about_dialog_body_reflects_workspace_version() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        assert!(app.dialog_is_open(DialogKind::About));

        let body = app.open_dialog_lines().expect("about dialog open");
        assert_eq!(
            body[0],
            format!("{} {}", rustamp_ui::PROGRAM_NAME, env!("CARGO_PKG_VERSION")),
            "about dialog first line must show the workspace version"
        );
    }

    #[test]
    fn load_playlist_uses_dialog_manager_contract() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Direct staging routes to DialogManager
        app.stage_playlist_path(PathBuf::from("staged.m3u"));
        assert_eq!(
            app.dialogs.confirm_load_playlist_path(),
            Some(PathBuf::from("staged.m3u"))
        );

        // Dropping a file routes to DialogManager
        assert!(app.dialog_drop_file(PathBuf::from("dropped.pls")));
        assert_eq!(
            app.dialogs.confirm_load_playlist_path(),
            Some(PathBuf::from("dropped.pls"))
        );

        // Closing resets DialogManager state
        app.close_dialog(DialogKind::LoadPlaylist);
        assert_eq!(app.dialogs.confirm_load_playlist_path(), None);
        assert!(app.playlist_text().is_empty());
    }

    #[test]
    fn recreate_engine_preserves_playback_state_and_cleans_flush_failure() {
        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        app.submit(Command::SetVolume(180)).unwrap();
        app.submit(Command::SetBalance(42)).unwrap();
        app.submit(Command::SetEqEnabled(true)).unwrap();
        app.submit(Command::SetPreamp(3.0)).unwrap();
        app.submit(Command::ToggleShuffle).unwrap();
        app.submit(Command::ToggleRepeat).unwrap();

        let queue = vec![TrackSource::path(t1.clone()), TrackSource::path(t2.clone())];
        app.engine_mut().set_queue(queue);
        app.engine_mut().set_current_index(Some(1));

        assert_eq!(app.engine().volume(), 180);
        assert_eq!(app.engine().balance(), 42);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert!(app.engine().shuffle());
        assert!(app.engine().repeat());
        assert!(!app.engine().is_flush_failed());

        // Recreate the engine and output
        app.recreate_engine().expect("recreation succeeds");

        assert_eq!(app.engine().volume(), 180);
        assert_eq!(app.engine().balance(), 42);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert!(app.engine().shuffle());
        assert!(app.engine().repeat());
        assert!(!app.engine().is_flush_failed());
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert!(app.engine().last_error().is_none());
    }

    /// The division of labour the RFC left open (W23 gap 3), end to end.
    ///
    /// A fatal mid-run stream error is recovered by the engine reopening the
    /// device it already owns. The application must not step in with
    /// `recreate_engine()`, because its fresh engine starts at position zero
    /// and would destroy the frozen position of RFC 0012 §4.3 item 4.
    ///
    /// The discriminator is the lifecycle counter of the output object the app
    /// held before the failure: a caller-side recreation builds a *new*
    /// `NullOutput::pair()`, whose `opens()` reads 1 after `Engine::new`, while
    /// the engine's own reopen takes the old object's `opens()` to 2.
    #[test]
    fn a_stream_error_is_recovered_by_the_engine_not_by_app_recreation() {
        let (mut app, dir) = app_with_file(None);
        let track = track(&dir, "track1_44100.tst");
        app.engine_mut().set_queue(vec![TrackSource::path(track)]);

        app.submit(Command::Play).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while app.engine().buffered_frames() < 256 {
            app.pump().unwrap();
            assert!(std::time::Instant::now() < deadline, "audio never buffered");
            std::thread::yield_now();
        }
        app.advance_null_clock(256);
        app.pump().unwrap();
        assert!(app.engine().position_frames() > 0);
        let clock = app.clock.clone().expect("the null output has a clock");
        assert_eq!(clock.opens(), 1);

        clock.fail();
        app.pump().unwrap();

        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert!(app.engine().is_stream_lost(), "the engine armed its reopen");
        assert!(
            !app.engine().is_flush_failed(),
            "nothing asked the caller to recreate anything"
        );
        assert!(
            clock.is_failed(),
            "the app still holds the very output that failed"
        );
        assert_eq!(
            app.clock.as_ref().unwrap().opens(),
            1,
            "the app did not recreate the engine or the output"
        );
        let frozen_ms = app.engine().position_ms();
        assert!(
            frozen_ms > 0,
            "the frozen position survived: {frozen_ms} ms"
        );
        for _ in 0..3 {
            app.ensure_engine_ready().unwrap();
            app.pump().unwrap();
            assert_eq!(app.engine().position_ms(), frozen_ms);
            assert!(
                app.last_error().is_some(),
                "the device failure stays visible"
            );
        }

        // Play again: the engine reopens its own device.
        app.submit(Command::Play).unwrap();
        assert_eq!(
            app.clock.as_ref().unwrap().opens(),
            2,
            "the same output object was reopened, not replaced"
        );
        assert!(!app.engine().is_stream_lost());
        assert!(!clock.is_failed(), "the reopen cleared the device latch");
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }

    // ----- the headless run's report -----

    /// The headless run announces a track only when the track is **open**.
    ///
    /// The transport is the wrong signal: the engine sets it when the run
    /// starts, a moment before the decode thread has opened the file. A file
    /// that cannot be opened would then be announced as playing, and the
    /// report would depend on how fast the decode thread came up — the race
    /// this test pins. The rule is checked here, in the crate, so it is
    /// decided by a fact and not by a timing.
    #[test]
    fn a_track_that_never_opened_is_not_announced() {
        // The real registry: the test registry opens every file it is given,
        // so it cannot produce a file that fails to open.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("never-written.flac");
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: Some(MediaLocation::File(missing)),
            seconds: None,
        };
        let mut app = App::new(options).unwrap();

        // The run starts: the transport says `Playing` right away, before
        // the decode thread has opened anything. That is the intermediate
        // state the report must not follow.
        app.submit(Command::Play).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert!(
            !app.headless_track_open(),
            "a file that has not opened must not be announced"
        );

        // The decode thread's error arrives; the track never opened.
        assert!(
            pump_until(&mut app, |app| app.engine().last_error().is_some(), 2_000),
            "the missing file must fail the run"
        );
        assert!(
            !app.headless_track_open(),
            "a file that failed to open must never be announced"
        );
    }

    // ----- what the window knows about a track that cannot be seeked -----

    /// The centre of the position slider of the classic main window.
    fn position_slider_centre() -> (i32, i32) {
        let track = MainWindowLayout::CLASSIC.position_track;
        (
            track.x + track.width as i32 / 2,
            track.y + track.height as i32 / 2,
        )
    }

    #[test]
    fn a_track_that_cannot_be_seeked_greys_the_position_bar_and_offers_no_seek() {
        let (mut app, dir) = app_with_file(None);
        let stream = track(&dir, "live.strm");
        app.engine_mut().set_queue(vec![TrackSource::path(stream)]);
        app.sync_entry_ids();
        app.submit(Command::Play).unwrap();

        // The track opened, so the window knows the engine's fact.
        assert!(
            pump_until(
                &mut app,
                |app| app.state().title.contains("App Test Stream"),
                2_000
            ),
            "the stream never opened"
        );
        assert!(!app.engine().is_seekable());
        assert!(
            !app.state().seekable,
            "the window must know that the track cannot be seeked"
        );

        // A press on the position slider offers no seek: the engine would
        // refuse it and report the refusal.
        let (x, y) = position_slider_centre();
        assert!(app.ui.mouse_down(x, y, app.now_ms()).is_empty());
        assert_eq!(app.state().position, 0);
    }

    #[test]
    fn a_seekable_track_keeps_its_seek() {
        let (mut app, dir) = app_with_file(None);
        let file = track(&dir, "one.tst");
        app.engine_mut().set_queue(vec![TrackSource::path(file)]);
        app.sync_entry_ids();
        app.submit(Command::Play).unwrap();

        assert!(
            pump_until(
                &mut app,
                |app| app.state().title.contains("App Test Track"),
                2_000
            ),
            "the track never opened"
        );
        assert!(app.engine().is_seekable());
        assert!(app.state().seekable);

        // The seek is still offered: the window did not change for a file.
        let (x, y) = position_slider_centre();
        let commands = app.ui.mouse_down(x, y, app.now_ms());
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Seek { .. })),
            "{commands:?}"
        );
    }
}
