// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Command routing (task card N1).
//!
//! A [`Command`] reaches the app from two places: the UI (mouse and keyboard
//! outcomes of [`rustamp_ui::MainWindow`]) and the command channel (plugins,
//! the platform layer). The app is the only owner of the player, so it decides
//! which layer acts:
//!
//! * the **audio engine** owns transport, seeking, volume/balance, the queue
//!   and "stop after current" — [`Engine::apply`](rustamp_audio::Engine::apply)
//!   already ignores everything else, so the app forwards **every** command to
//!   it and lets it pick;
//! * the **application layer** owns the windows, the skin, the mode flags
//!   (shuffle/repeat — the engine deliberately does not track them, RFC 0001 /
//!   S15), the playlist model and the dialogs. [`app_owns`] says which commands
//!   those are, and [`crate::app::App`] applies them.
//!
//! [`app_owns`] is an exhaustive match on purpose: adding a `Command` variant
//! must force a routing decision instead of silently falling into a catch-all.

use rustamp_core::command::Command;
use rustamp_ui::ShortcutAction;

/// Whether the application layer must act on `command` itself.
///
/// `false` means the audio engine is the only owner (the app still forwards it,
/// the engine just does not ignore it).
pub fn app_owns(command: &Command) -> bool {
    use Command as C;
    match command {
        // ----- the audio engine's own surface -----
        C::Play
        | C::Pause
        | C::TogglePause
        | C::Stop
        | C::StopWithFadeout
        | C::Next
        | C::Prev
        | C::Seek { .. }
        | C::SeekRelative { .. }
        | C::SetVolume(_)
        | C::SetBalance(_)
        | C::PlayIndex(_)
        | C::SkipBy { .. }
        | C::PlayLast
        | C::ToggleStopAfterCurrent => false,

        // ----- everything the app owns -----
        C::ToggleShuffle
        | C::ToggleRepeat
        | C::SetShuffle(_)
        | C::SetRepeat(_)
        | C::AddItems { .. }
        | C::Remove(_)
        | C::Clear
        | C::MoveEntries { .. }
        | C::Select(_)
        | C::PlayEntry(_)
        | C::Sort(_)
        | C::Randomize
        | C::Reverse
        | C::SetBand { .. }
        | C::SetPreamp(_)
        | C::SetEqEnabled(_)
        | C::SetEqAuto(_)
        | C::ToggleWindow(_)
        | C::ToggleDoubleSize
        | C::ToggleWindowShade(_)
        | C::ToggleAlwaysOnTop
        | C::LoadSkin(_)
        | C::Quit
        | C::OpenPrefs
        | C::OpenMedia { .. }
        | C::OpenJump { .. }
        | C::OpenFileInfo
        | C::OpenPrefsPage(_)
        | C::OpenAbout
        | C::OpenSkinBrowser
        | C::RefreshSkin
        | C::OpenBookmarkEditor
        | C::AddBookmark
        | C::ToggleEasyMove
        | C::Minimize
        | C::FocusNextWindow
        | C::ShowMainWindow
        | C::CloseFocusedWindow
        | C::OpenMainMenu
        | C::ConfigureVisPlugin
        | C::ToggleVisPlugin
        | C::OpenPlaylist
        | C::OpenOptionsMenu
        | C::OpenVisualizationMenu
        | C::SavePlaylist(_) => true,
    }
}

/// The log line for an S03 action that has no [`Command`] yet.
///
/// Task card D3 left about 30 actions as `ShortcutOutcome::Unhandled`; the card
/// asks the app to log them **with their command id**, which is what this
/// builds. RFC 0005 added commands for most of them, so the remaining set is
/// the deferred minibrowser/video-fullscreen/playlist-search group plus
/// `NewInstance` (app-local).
pub fn unhandled_message(action: ShortcutAction) -> String {
    format!(
        "unhandled shortcut action {action:?} (command id {}): no Command yet",
        action.command_id()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::playlist::{EntryId, SortKey};
    use rustamp_core::state::WindowId;
    use rustamp_ui::ShortcutAction;

    #[test]
    fn the_engine_owns_the_transport_and_the_queue() {
        for command in [
            Command::Play,
            Command::Pause,
            Command::TogglePause,
            Command::Stop,
            Command::StopWithFadeout,
            Command::Next,
            Command::Prev,
            Command::Seek { to_ms: 1000 },
            Command::SeekRelative { delta_ms: -5000 },
            Command::SetVolume(128),
            Command::SetBalance(-10),
            Command::PlayIndex(2),
            Command::SkipBy { delta: 10 },
            Command::PlayLast,
            Command::ToggleStopAfterCurrent,
        ] {
            assert!(!app_owns(&command), "{command:?} belongs to the engine");
        }
    }

    #[test]
    fn the_app_owns_modes_windows_skin_and_dialogs() {
        for command in [
            Command::ToggleShuffle,
            Command::ToggleRepeat,
            Command::SetShuffle(true),
            Command::SetRepeat(false),
            Command::ToggleWindow(WindowId::Equalizer),
            Command::ToggleDoubleSize,
            Command::ToggleWindowShade(WindowId::Main),
            Command::ToggleAlwaysOnTop,
            Command::LoadSkin("x.wsz".into()),
            Command::Quit,
            Command::OpenPrefs,
            Command::OpenFileInfo,
            Command::OpenAbout,
            Command::RefreshSkin,
            Command::OpenPlaylist,
            Command::ToggleEasyMove,
            Command::Minimize,
            Command::ShowMainWindow,
            Command::CloseFocusedWindow,
            Command::OpenMainMenu,
            Command::OpenOptionsMenu,
            Command::OpenVisualizationMenu,
            Command::ToggleVisPlugin,
            Command::SavePlaylist("list.m3u".into()),
        ] {
            assert!(app_owns(&command), "{command:?} belongs to the app");
        }
    }

    #[test]
    fn the_app_owns_the_playlist_model() {
        for command in [
            Command::AddItems {
                items: Vec::new(),
                at: None,
                play: true,
            },
            Command::Remove(vec![EntryId(1)]),
            Command::Clear,
            Command::MoveEntries {
                ids: vec![EntryId(1)],
                to: 0,
            },
            Command::Select(rustamp_core::command::SelectOp::All),
            Command::PlayEntry(EntryId(3)),
            Command::Sort(SortKey::Title),
            Command::Randomize,
            Command::Reverse,
        ] {
            assert!(app_owns(&command), "{command:?} belongs to the app");
        }
    }

    #[test]
    fn the_app_owns_the_equalizer_until_the_engine_has_a_stage() {
        for command in [
            Command::SetBand {
                band: 0,
                gain_db: 3.0,
            },
            Command::SetPreamp(1.0),
            Command::SetEqEnabled(true),
            Command::SetEqAuto(false),
        ] {
            assert!(app_owns(&command), "{command:?} belongs to the app for now");
        }
    }

    #[test]
    fn the_unhandled_message_carries_the_command_id() {
        let message = unhandled_message(ShortcutAction::NewInstance);
        assert!(message.contains("40305"), "{message}");
        assert!(message.contains("NewInstance"), "{message}");

        let message = unhandled_message(ShortcutAction::PlaylistSearch);
        assert!(message.contains("40318"), "{message}");
    }
}
