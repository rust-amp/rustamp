// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The player window's external-control adapter, from S10 only.
//!
//! This is not the plugin host's IPC surface. Queries read the player's latest
//! published UI-thread snapshot; actions enter the existing app/shortcut path.
//! Unsettled calls are unsupported (1), not fabricated reference replies. In
//! particular S10 §13.2 leaves playing time/info open and §12 leaves title
//! pointers open. See `crates/rustamp-app/EXTERNAL_CONTROL.md` for coverage.

use rustamp_core::command::Command;
use rustamp_core::state::{PlayerState, TransportState};
use rustamp_plugin_abi::common::{WINAMP_VERSION_295, WM_WA_IPC};
use rustamp_ui::ShortcutAction;
use rustamp_ui::shortcuts::{GLOBAL_TABLE, MAIN_TABLE};

const WM_COMMAND: u32 = 0x0111;
#[cfg(windows)]
const UNSUPPORTED: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Query {
    Playing,
    OutputTime(u32),
    SeekStopped,
    ModuleName,
    Volume,
    Pan,
    ListLength,
    ListPosition,
    StreamInfo(u32),
    Shuffle,
    Repeat,
    FullStop,
}

#[derive(Debug, Clone, PartialEq)]
enum Request {
    Reply(u32),
    Query(Query),
    Shortcut(ShortcutAction),
    /// Only posted IPC commands: S10 did not read their sent return values.
    PostedCommand(Command),
    Unsupported,
    NotOurs,
}

/// Decode classic 32-bit parameters even on the 64-bit player. A caller may
/// either sign-extend or zero-extend -666; both carry the same classic value.
fn decode(message: u32, wparam: usize, lparam: isize) -> Request {
    if message == WM_COMMAND {
        let id = (wparam & 0xffff) as u32;
        return MAIN_TABLE
            .iter()
            .chain(GLOBAL_TABLE)
            .find(|binding| binding.action.command_id() == id)
            .map_or(Request::NotOurs, |binding| {
                Request::Shortcut(binding.action)
            });
    }
    if message != WM_WA_IPC {
        return Request::NotOurs;
    }
    let argument = wparam as u32 as i32;
    let query = match lparam as u32 {
        0 => return Request::Reply(WINAMP_VERSION_295),
        102 => return Request::PostedCommand(Command::Play),
        104 => Query::Playing,
        105 if (0..=2).contains(&argument) => Query::OutputTime(argument as u32),
        106 => Query::SeekStopped,
        109 => Query::ModuleName,
        122 if argument == -666 => Query::Volume,
        122 if (0..=255).contains(&argument) => {
            return Request::PostedCommand(Command::SetVolume(argument as u8));
        }
        123 if argument == -666 => Query::Pan,
        // Only these pan inputs were measured (S10 §5.6 / E9).
        123 if matches!(argument, 0 | 64 | 255) => {
            return Request::PostedCommand(Command::SetBalance(argument as u8 as i8));
        }
        124 => Query::ListLength,
        125 => Query::ListPosition,
        126 if (0..=2).contains(&argument) => Query::StreamInfo(argument as u32),
        127 if argument == 99 => return Request::Reply(0),
        250 => Query::Shuffle,
        251 => Query::Repeat,
        252 if matches!(argument, 0 | 1) => {
            return Request::PostedCommand(Command::SetShuffle(argument == 1));
        }
        253 if matches!(argument, 0 | 1) => {
            return Request::PostedCommand(Command::SetRepeat(argument == 1));
        }
        400 => Query::FullStop,
        // S10 §3: settled numeric replies; do not infer features from the
        // job-derived names (in particular 502 and 999).
        502 | 666 | 801 | 999 | 3015 | 3025 | 65536 => return Request::Reply(1),
        _ => return Request::Unsupported,
    };
    Request::Query(query)
}

/// `None` is a spec gap, not a reference reply. Never convert a pointer-shaped
/// measurement into an address in this process.
fn answer(query: Query, state: &PlayerState) -> Option<u32> {
    let fresh = state.transport == TransportState::Stopped
        && state.playlist_len == 0
        && state.stream.is_none();
    match query {
        Query::Playing => Some(match state.transport {
            TransportState::Stopped => 0,
            TransportState::Playing => 1,
            TransportState::Paused => 3,
        }),
        Query::OutputTime(0 | 1) if fresh => Some(u32::MAX),
        Query::OutputTime(2) if fresh => Some(0),
        Query::SeekStopped if state.transport == TransportState::Stopped => Some(u32::MAX),
        Query::ModuleName | Query::FullStop if fresh => Some(0),
        Query::Volume => Some(u32::from(state.volume)),
        Query::Pan => Some(i32::from(state.balance) as u32),
        Query::ListLength => Some(state.playlist_len),
        Query::ListPosition => Some(state.current_index.unwrap_or(0)),
        Query::StreamInfo(0..=2) if fresh => Some(0),
        Query::Shuffle => Some(u32::from(state.shuffle)),
        Query::Repeat => Some(u32::from(state.repeat)),
        _ => None,
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // Win32 subclass boundary only; the rest of the app denies unsafe.
pub(crate) mod native;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_id_is_lparam_and_arguments_are_classic_signed_32_bit() {
        assert_eq!(decode(WM_WA_IPC, 123, 0), Request::Reply(0x2905));
        for id in [122, 123] {
            let expected = if id == 122 { Query::Volume } else { Query::Pan };
            assert_eq!(
                decode(WM_WA_IPC, (-666_i32) as usize, id),
                Request::Query(expected)
            );
            assert_eq!(
                decode(WM_WA_IPC, (-666_i32) as u32 as usize, id),
                Request::Query(expected)
            );
        }
        assert_eq!(decode(0x0402, 0, 0), Request::NotOurs);
    }

    #[test]
    fn wm_command_reuses_the_entire_existing_main_and_global_tables() {
        for binding in MAIN_TABLE.iter().chain(GLOBAL_TABLE) {
            let wparam = (0x1234 << 16) | binding.action.command_id() as usize;
            assert_eq!(
                decode(WM_COMMAND, wparam, 99),
                Request::Shortcut(binding.action)
            );
        }
        assert_eq!(decode(WM_COMMAND, 123, 0), Request::NotOurs);
        assert_eq!(
            decode(WM_COMMAND, 40045, 0),
            Request::Shortcut(ShortcutAction::Play)
        );
    }

    #[test]
    fn measured_unknown_ids_answer_one() {
        for id in [1, 9999, -1, 2147483647, 65536, 801, 3015, 3025] {
            assert!(matches!(
                decode(WM_WA_IPC, 0, id),
                Request::Unsupported | Request::Reply(1)
            ));
        }
    }

    #[test]
    fn live_queries_use_player_values_not_reference_defaults() {
        let mut state = PlayerState {
            transport: TransportState::Paused,
            volume: 173,
            balance: -1,
            playlist_len: 60,
            current_index: Some(17),
            shuffle: true,
            repeat: true,
            ..PlayerState::default()
        };
        for (id, argument, expected) in [
            (104, 0, 3),
            (122, -666, 173),
            (123, -666, u32::MAX),
            (124, 0, 60),
            (125, 0, 17),
            (250, 0, 1),
            (251, 0, 1),
        ] {
            let Request::Query(query) = decode(WM_WA_IPC, argument as usize, id) else {
                panic!("query {id}")
            };
            assert_eq!(answer(query, &state), Some(expected));
        }
        for (transport, expected) in [(TransportState::Stopped, 0), (TransportState::Playing, 1)] {
            state.transport = transport;
            assert_eq!(answer(Query::Playing, &state), Some(expected));
        }
    }

    #[test]
    fn provisional_time_and_stream_replies_stay_within_measured_state() {
        let mut state = PlayerState::default();
        assert_eq!(answer(Query::OutputTime(0), &state), Some(u32::MAX));
        assert_eq!(answer(Query::OutputTime(1), &state), Some(u32::MAX));
        assert_eq!(answer(Query::OutputTime(2), &state), Some(0));
        for mode in 0..=2 {
            assert_eq!(answer(Query::StreamInfo(mode), &state), Some(0));
        }
        state.transport = TransportState::Playing;
        for mode in 0..=2 {
            assert_eq!(answer(Query::OutputTime(mode), &state), None);
            assert_eq!(answer(Query::StreamInfo(mode), &state), None);
        }
        assert_eq!(answer(Query::SeekStopped, &state), None);
        for id in [127, 128, 211, 212, 214, 242, 260, 261, 281, 800, 900] {
            assert_eq!(decode(WM_WA_IPC, 0, id), Request::Unsupported);
        }
    }

    #[test]
    fn posted_effects_map_to_existing_commands_without_guessing_sent_replies() {
        for (id, argument, command) in [
            (102, 0, Command::Play),
            (122, 64, Command::SetVolume(64)),
            (123, 255, Command::SetBalance(-1)),
            (252, 1, Command::SetShuffle(true)),
            (253, 0, Command::SetRepeat(false)),
        ] {
            assert_eq!(
                decode(WM_WA_IPC, argument, id),
                Request::PostedCommand(command)
            );
        }
        assert_eq!(
            decode(WM_WA_IPC, (-666_i32) as usize, 252),
            Request::Unsupported
        );
        assert_eq!(decode(WM_WA_IPC, 32, 123), Request::Unsupported);
    }
}
