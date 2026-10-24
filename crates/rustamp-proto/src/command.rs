//! The [`Command`] enum: every user or plugin intent that mutates the player or
//! its windows and dialogs.
//!
//! Commands are *requests*; the app/engine applies them and answers with
//! [`Event`](crate::event::Event)s. They are `serde`-serializable so that the
//! plugin host can forward them over IPC. Indices are `u32`; entries are
//! referenced by stable [`EntryId`](crate::playlist::EntryId) where possible.
//!
//! **Append rule** (RFC 0005 §3): new variants go at the **end** of [`Command`],
//! never into the logical sections, because `postcard` encodes an enum by its
//! declaration index — appending keeps every existing variant's wire bytes
//! unchanged.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::playlist::{EntryId, MediaLocation, SortKey};
use crate::state::WindowId;

/// The kind of a plug-in that [`Command::ConfigurePlugin`] names (RFC 0020
/// §3.3, option A).
///
/// This is the **ABI's** kind type, not a mirror of it: the payload and the
/// protocol's `LoadPlugin`/`PluginLoaded` messages therefore carry one kind
/// type and one encoding. Naming it here is the reason this crate takes exactly
/// one internal dependency (`rustamp-plugin-abi`, `MIT OR Apache-2.0`); see the
/// RFC 0004 §3 erratum (RFC 0004 §7) and RFC 0020 §7.
pub use rustamp_plugin_abi::PluginKind;

/// Selection operations on the playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectOp {
    /// Replace the selection with exactly these entries.
    Set(Vec<EntryId>),
    /// Toggle one entry.
    Toggle(EntryId),
    /// Select all.
    All,
    /// Select none.
    None,
    /// Invert.
    Invert,
}

/// Which kind of media [`Command::OpenMedia`] asks the user for.
///
/// Provisional (RFC 0005 §5.2): exactly the three kinds S03 records a public
/// command id for; the full dialog list belongs to S04 (not started).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenMediaKind {
    /// A file picker ("Open file(s)", command id 40029).
    Files,
    /// A URL entry ("Open location", command id 40185).
    Location,
    /// A directory picker ("Open directory", command id 40187).
    Directory,
}

/// Which jump dialog [`Command::OpenJump`] opens.
///
/// Confirming the dialog produces an existing command: [`Command::PlayEntry`]
/// for [`JumpTarget::File`], [`Command::Seek`] for [`JumpTarget::Time`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JumpTarget {
    /// Jump to a playlist entry (command id 40194).
    File,
    /// Jump to a playback position (command id 40193).
    Time,
}

/// A preferences page [`Command::OpenPrefsPage`] opens.
///
/// Provisional: the full page list belongs to S04 (not started); the two values
/// defined now are the ones S03 records command ids for. Both are measured
/// (2026-10-15): 40191 opens the dialog at Plug-ins > Visualization, 40317 at
/// Options > Visualization. Growing the enum follows the append rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrefsPage {
    /// Plug-ins > Visualization ("Select visualization plug-in", id 40191).
    VisPlugin,
    /// Options > Visualization ("Visualization options", id 40317).
    VisOptions,
}

/// A command to the player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    // ----- transport -----
    /// Play (resume if paused, restart current if playing).
    Play,
    /// Pause (no-op unless playing).
    Pause,
    /// Toggle between playing and paused.
    TogglePause,
    /// Stop.
    Stop,
    /// Stop with a fade-out (duration from config).
    StopWithFadeout,
    /// Next track.
    Next,
    /// Previous track.
    Prev,
    /// Seek to an absolute position in ms.
    Seek {
        /// Target position in ms.
        to_ms: u64,
    },
    /// Seek relative to the current position (negative = backwards).
    SeekRelative {
        /// Delta in ms.
        delta_ms: i64,
    },
    /// Set volume, `0..=255` (Winamp style).
    SetVolume(u8),
    /// Set balance, `-127..=127`; values outside are clamped.
    SetBalance(i8),
    /// Toggle shuffle.
    ToggleShuffle,
    /// Toggle repeat.
    ToggleRepeat,
    /// Set shuffle explicitly.
    SetShuffle(bool),
    /// Set repeat explicitly.
    SetRepeat(bool),

    // ----- playlist -----
    /// Add files/URLs. `at = None` appends. `play` starts the first added entry.
    AddItems {
        /// Locations to add.
        items: Vec<MediaLocation>,
        /// Insert position (clamped); `None` = end.
        at: Option<u32>,
        /// Start playing the first new entry.
        play: bool,
    },
    /// Remove entries.
    Remove(Vec<EntryId>),
    /// Remove all entries.
    Clear,
    /// Move entries as a block; the first lands at `to` in the list without the moved entries.
    MoveEntries {
        /// Entries to move.
        ids: Vec<EntryId>,
        /// Target index.
        to: u32,
    },
    /// Selection change.
    Select(SelectOp),
    /// Play the entry at an index.
    PlayIndex(u32),
    /// Play a specific entry.
    PlayEntry(EntryId),
    /// Sort the playlist.
    Sort(SortKey),
    /// Randomize the order.
    Randomize,
    /// Reverse the order.
    Reverse,

    // ----- equalizer -----
    /// Set one band (`band < EQ_BANDS`) gain in dB (clamped to +-12).
    SetBand {
        /// Band index `0..10`.
        band: u8,
        /// Gain in dB.
        gain_db: f32,
    },
    /// Set preamp gain in dB.
    SetPreamp(f32),
    /// Enable/disable EQ.
    SetEqEnabled(bool),
    /// Enable/disable EQ auto-load.
    SetEqAuto(bool),

    // ----- windows / UI -----
    /// Show/hide a window.
    ToggleWindow(WindowId),
    /// Toggle double-size mode.
    ToggleDoubleSize,
    /// Toggle windowshade for a window.
    ToggleWindowShade(WindowId),
    /// Toggle always-on-top.
    ToggleAlwaysOnTop,
    /// Load a skin from a `.wsz`/`.zip` or folder.
    LoadSkin(PathBuf),

    // ----- app -----
    /// Quit the application.
    Quit,
    /// Open the preferences dialog.
    OpenPrefs,

    // ----- RFC 0005 app-level additions (append-only; see the module docs) -----
    //
    // The ids in the doc lines are the public `WM_COMMAND` ids S03 records;
    // they are interoperability facts and are also the plugin host's id table.
    /// Move the current entry by `delta` positions in the order `Next`/`Prev`
    /// follow, clamped to the playlist; `delta = 0` and an empty playlist are
    /// no-ops. While stopped the current entry moves without starting playback
    /// (S02 §1). Ids 40195 (+10) and 40197 (-10). The play order (shuffle) vs
    /// playlist order question is open (RFC 0005 O9 / S15).
    SkipBy {
        /// Signed step in entries; `+10` = "10 tracks forward", `-10` = back.
        delta: i32,
    },
    /// Play the last entry of the playlist; a no-op on an empty playlist. The
    /// engine resolves the last entry, so the caller needs no length. This is
    /// the S03 §5 mouse action "end of list" (Ctrl+click the next button); S03
    /// records no command id for it, so the plugin host never sees it (O15).
    PlayLast,
    /// Toggle "stop after current track": when the current track ends the
    /// transport stops instead of advancing to the next entry. Id 40157.
    ToggleStopAfterCurrent,
    /// Open the player's open-file / open-location / open-directory dialog; on
    /// confirmation the app adds the chosen media with [`Command::AddItems`].
    /// Ids 40029 (files), 40185 (location), 40187 (directory).
    OpenMedia {
        /// Which dialog to open.
        kind: OpenMediaKind,
        /// Start playing the first added entry. `true` for the main-window
        /// actions; the playlist-window add commands (1032/1036/1039) are
        /// deferred and their value is unmeasured (RFC 0005 O5).
        play: bool,
    },
    /// Open the jump dialog (to a playlist entry or to a position). Ids 40194
    /// (file) and 40193 (time).
    OpenJump {
        /// Which jump dialog.
        target: JumpTarget,
    },
    /// Open the file-info box for the current track. Id 40188. No payload: the
    /// target is always the current entry, which the app resolves from state.
    OpenFileInfo,
    /// Open the preferences dialog at a specific page. Id 40191 (visualization
    /// plug-in) and 40317 (visualization options); see [`PrefsPage`].
    OpenPrefsPage(PrefsPage),
    /// Open the about box. Id 40041.
    OpenAbout,
    /// Open the skin browser; the user's pick becomes a [`Command::LoadSkin`].
    /// Id 40219.
    OpenSkinBrowser,
    /// Re-apply the skin the app last reported through
    /// [`Event::SkinChanged`](crate::event::Event::SkinChanged). No payload:
    /// the app already tracks the current skin to render it, and the UI machine
    /// does not know the path. Id 40291.
    RefreshSkin,
    /// Open the bookmark-management dialog. Id 40320. The bookmark store does
    /// not exist yet (RFC 0005 O12); the handling lands with that feature.
    OpenBookmarkEditor,
    /// Add the current track to the bookmark list. Id 40321. Same open point as
    /// [`Command::OpenBookmarkEditor`].
    AddBookmark,
    /// Toggle easy-move (a larger drag target for moving the player's windows).
    /// Id 40186. Symmetric with [`Command::ToggleAlwaysOnTop`]; the platform
    /// layer applies it.
    ToggleEasyMove,
    /// Minimize the player's **main** window (owner decision O4). Id 40334.
    Minimize,
    /// Move keyboard focus to the next player window, in [`WindowId`] order.
    /// Id 40063.
    FocusNextWindow,
    /// Show (un-minimize, raise) and focus the main window. Id 40258.
    /// Deliberately one-way: [`Command::ToggleWindow`] would *hide* it.
    ShowMainWindow,
    /// Close whichever player window has focus. Id 40224. The equalizer and
    /// playlist windows hide (they are toggle windows); closing the main window
    /// quits (owner decision O3).
    CloseFocusedWindow,
    /// Open the main-menu popup. Id 40043. Handling is deferred to S04.
    OpenMainMenu,
    /// Ask the loaded visualization plug-in to show its configuration. Id
    /// 40221. Applied by the app *through* the plugin host, so it travels to the
    /// host as a `Message::CommandToHost` — handling is deferred to M3.
    ConfigureVisPlugin,
    /// Start/stop the visualization plug-in (whether vis data flows). Id 40192.
    /// Same host bridge and deferral as [`Command::ConfigureVisPlugin`].
    ToggleVisPlugin,
    /// Open the load-playlist dialog (S06 §4, LIST OPTS button); on
    /// confirmation the app loads the chosen playlist file. No payload: the
    /// path comes from the user's pick, inside the app (RFC 0010). The spec
    /// fixes the dialog's title only, so replace-vs-append and every other
    /// confirmation semantic stays open.
    OpenPlaylist,
    /// Open the Options popup (clutter-bar letter O, S02 §2, S04 §4). RFC 0016.
    OpenOptionsMenu,
    /// Open the visualization menu (clutter-bar letter V, S02 §2, S04 §5).
    /// RFC 0016.
    OpenVisualizationMenu,
    /// Write the current playlist to the given file. The format follows the
    /// extension (`.m3u`, `.m3u8`, `.pls`, `.xspf`); any other extension is
    /// refused. S03 records command id 40204 for it. RFC 0017.
    SavePlaylist(PathBuf),
    /// Remove every entry whose local file no longer exists on disk ("Remove
    /// all dead files", Alt+Delete, S03 §2, command id 40222). Network streams
    /// ([`MediaLocation::Url`](crate::playlist::MediaLocation::Url)) are never
    /// checked and always survive; the spec fixes the label only, so the
    /// stream rule is an own choice (RFC 0018). No payload: the app owns the
    /// player's state and resolves the dead entries itself. RFC 0018.
    RemoveDeadFiles,

    // ----- RFC 0020: the plug-in requests the player sends to the host -----
    //
    // Only the host's UI thread may call into a plug-in (S09 §3.5), so these
    // five requests travel to `rustamp-plughost32.exe` inside
    // `Message::CommandToHost` — the carrier that already exists; `Message`
    // itself is unchanged. They are **player-internal**: no public
    // `WM_COMMAND` id exists for a request that names a plug-in, so the host's
    // id table has no row for them and the host never forwards them back
    // (RFC 0020 §3.1, §12).
    /// Ask the host to open the configuration dialog of the plug-in with this
    /// id. The id is the one the host assigned in its `PluginLoaded` answer and
    /// is valid only for that host process's lifetime. `kind` is the kind the
    /// request claims; the host compares it with the kind it recorded when the
    /// plug-in was loaded and refuses a mismatch. RFC 0020.
    ConfigurePlugin {
        /// Host-assigned plug-in id.
        plugin_id: u32,
        /// The kind the request claims the plug-in has.
        kind: PluginKind,
    },
    /// Ask the host to seek the input plug-in with this id. The plug-in flushes
    /// its own output at the new time (S09 §3.7); the host must not. RFC 0020.
    SeekPlugin {
        /// Host-assigned plug-in id (valid for this host process only).
        plugin_id: u32,
        /// Target position in milliseconds, passed through unchanged (S09 sets
        /// no range).
        to_ms: i32,
    },
    /// Ask the host to pause the input plug-in with this id (S09 §3.1 offset
    /// 0x3c; the plug-in pauses its output). RFC 0020.
    PausePlugin {
        /// Host-assigned plug-in id (valid for this host process only).
        plugin_id: u32,
    },
    /// The counterpart of [`Command::PausePlugin`] (offset 0x40). RFC 0020.
    UnpausePlugin {
        /// Host-assigned plug-in id (valid for this host process only).
        plugin_id: u32,
    },
    /// Ask the host to stop the input plug-in with this id; the plug-in closes
    /// its own output (S09 §3.5 step 6). RFC 0020.
    StopPlugin {
        /// Host-assigned plug-in id (valid for this host process only).
        plugin_id: u32,
    },
}
