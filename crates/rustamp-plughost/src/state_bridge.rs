//! Host side of the player-state bridge (RFC 0006 out-of-process hosting).
//!
//! The app (64-bit) owns the real player; the host (i686) owns the fake
//! `Winamp v1.x` window that plugins query through [`IpcSurface`]. The only
//! state channel the [`rustamp_ipc::Message`] protocol provides in that
//! direction is [`Message::PlayerStateUpdate`], which carries a
//! [`PlayerState`] snapshot. [`apply_player_state`] copies that snapshot into
//! the surface fields so a plugin's query reflects the real player rather
//! than a default; [`apply_message`] is the single entry point the host loop
//! calls for every inbound protocol message.
//!
//! # Mapping (message -> surface field -> measured id)
//!
//! One place, one row per field. Each comment names the measured classic id
//! that reads the field (ids from `src/ipc.rs`, measured per spec S10).
//!
//! * `state.transport` -> `surface.is_playing` (id 104 `IPC_ISPLAYING`; also
//!   gates the playing-reference answers of 105, 106, 126 and 501).
//! * `state.position_ms` -> `surface.output_position_ms` (id 105 mode 0
//!   `IPC_GETOUTPUTTIME` while playing).
//! * `state.duration_ms` -> `surface.output_length_ms` (id 105 modes 1/2;
//!   `None` means unknown, stored as 0).
//! * `state.volume` -> `surface.volume` (id 122 `IPC_SETVOLUME`, unsigned).
//! * `state.balance` -> `surface.pan` (id 123 `IPC_SETPANNING`, signed;
//!   direct `i8` -> `i32` widening).
//! * `state.playlist_len` -> `surface.playlist_len` (id 124
//!   `IPC_GETLISTLENGTH`).
//! * `state.current_index` -> `surface.playlist_pos` (id 125 `IPC_GETLISTPOS`;
//!   `None` means no current entry, stored as 0 like the fresh default).
//! * `state.stream.sample_rate_hz` -> `surface.sample_rate` (id 126 mode 0
//!   `IPC_GETINFO` while playing; `None` stream means 0).
//! * `state.stream.bitrate_kbps` -> `surface.bitrate` (id 126 mode 1;
//!   `None` stream or `None` bitrate means 0).
//! * `state.stream.channels` -> `surface.channels` (id 126 mode 2;
//!   `None` stream means 0).
//! * `state.eq.bands` -> `surface.eq_bands` (id 127 selectors 0..=9
//!   `IPC_GETEQDATA`, written via 128; dB converted, see
//!   [`eq_classic_from_db`]).
//! * `state.eq.preamp` -> `surface.eq_preamp` (id 127 selector 10; same
//!   conversion).
//! * `state.eq.enabled` -> `surface.eq_enabled` (id 127 selector 11).
//! * `state.eq.auto` -> `surface.eq_autoload` (id 127 selector 12).
//! * `state.shuffle` -> `surface.shuffle` (ids 250/252 shuffle read/set).
//! * `state.repeat` -> `surface.repeat` (ids 251/253 repeat read/set).
//!
//! `state.stop_after_current` has no surface field and no measured id reads
//! it, so it is ignored here.
//!
//! # State the protocol does not carry (finding, not a gap in the mapping)
//!
//! No existing protocol message carries these, so this module invents no
//! message and writes no made-up value; the surface keeps whatever the host
//! set locally (or its documented default / "needs a playing reference"
//! behaviour from `src/ipc.rs`):
//!
//! * Window visibility (`surface.visible_*`, ids 261/900): nothing in
//!   `PlayerState` or any other `Message` variant carries it.
//! * Runtime handles (`surface.{eq,playlist,minibrowser,video,main}_hwnd`,
//!   `surface.menu_handle`, ids 260/281): host-local values owned by the host
//!   window itself (`src/host_window.rs` publishes `main_hwnd` at creation);
//!   they must never cross from the app.
//! * Internet availability (`surface.inet_available`, id 242): no protocol field.
//! * EQ slot selector (`surface.eq_selected`, the 127-selects/128-writes
//!   latch): local to one plugin's 127/128 interaction, not player state.
//! * Seek result (id 106) and video-playing state (id 501): `PlayerState` has
//!   transport and position but no seek-ack and no video flag, so both ids
//!   keep their `NeedsPlayingReference`/default answers while playing or
//!   stopped respectively.
//! * Module/spectrum entry points (ids 109, 800, 801, 3015, 3025): these need
//!   a plugin DLL in process, not player state.
//!
//! If the contract has to grow, that is an RFC decision; this module reports
//! the need and changes no existing protocol message.

use crate::ipc::{EQ_BAND_COUNT, IpcSurface};
use rustamp_ipc::Message;
use rustamp_proto::state::{PlayerState, TransportState};

/// Convert one equalizer gain from the protocol domain (dB, nominally
/// `-12.0..=12.0`) to the classic surface domain (`0..=63`, `i32`).
///
/// Anchor points from spec S09 (audited): `0 = +12 dB`, `31 = 0 dB`,
/// `63 = -12 dB`. The linear law through the endpoints is
/// `(12 - db) * 63 / 24`, which is exactly `31.5` at `0 dB`; the `floor`
/// quantizes the centre down to the documented `31` so all three anchors hold
/// exactly. Out-of-range gains saturate to `0`/`63`. Non-finite input cannot
/// be mapped honestly and yields the flat value `31`.
pub fn eq_classic_from_db(db: f32) -> i32 {
    if !db.is_finite() {
        return 31;
    }
    ((12.0_f32 - db) * 63.0_f32 / 24.0_f32)
        .floor()
        .clamp(0.0, 63.0) as i32
}

/// Map the protocol transport to the classic playing value the surface stores
/// (S10/`src/ipc.rs`: 0 stopped, 1 playing, 3 paused; read by id 104).
pub fn classic_playing_value(transport: TransportState) -> u32 {
    match transport {
        TransportState::Stopped => 0,
        TransportState::Playing => 1,
        TransportState::Paused => 3,
    }
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Apply one [`PlayerState`] snapshot to the surface. See the module docs for
/// the field-by-field mapping and its measured ids.
pub fn apply_player_state(surface: &mut IpcSurface, state: &PlayerState) {
    // Id 104 IPC_ISPLAYING (also gates 105, 106, 126, 501).
    surface.is_playing = classic_playing_value(state.transport);
    // Id 105 mode 0 IPC_GETOUTPUTTIME (position while playing).
    surface.output_position_ms = saturating_u32(state.position_ms);
    // Id 105 modes 1/2 (length while playing; None = unknown = 0).
    surface.output_length_ms = state.duration_ms.map(saturating_u32).unwrap_or(0);
    // Id 122 IPC_SETVOLUME (unsigned 0..=255; both domains already are).
    surface.volume = state.volume;
    // Id 123 IPC_SETPANNING (signed; i8 widens to i32 exactly).
    surface.pan = i32::from(state.balance);
    // Id 124 IPC_GETLISTLENGTH.
    surface.playlist_len = state.playlist_len;
    // Id 125 IPC_GETLISTPOS (None = no current entry = 0, the fresh default).
    surface.playlist_pos = state.current_index.unwrap_or(0);
    // Id 126 IPC_GETINFO modes 0/1/2 (None stream, or None bitrate, reads 0).
    surface.sample_rate = state.stream.map(|s| s.sample_rate_hz).unwrap_or(0);
    surface.bitrate = state.stream.and_then(|s| s.bitrate_kbps).unwrap_or(0);
    surface.channels = state.stream.map(|s| u32::from(s.channels)).unwrap_or(0);
    // Ids 127/128 IPC_GETEQDATA/IPC_SETEQDATA (dB -> classic 0..=63).
    debug_assert_eq!(state.eq.bands.len(), EQ_BAND_COUNT);
    for (slot, gain) in surface.eq_bands.iter_mut().zip(state.eq.bands.iter()) {
        *slot = eq_classic_from_db(*gain);
    }
    // Id 127 selector 10 (preamp shares the band ramp: S01 captures behave as
    // a band; whether the ramps are one ramp is still an S07 open item).
    surface.eq_preamp = eq_classic_from_db(state.eq.preamp);
    // Id 127 selectors 11/12.
    surface.eq_enabled = state.eq.enabled;
    surface.eq_autoload = state.eq.auto;
    // Ids 250/252 shuffle, 251/253 repeat.
    surface.shuffle = state.shuffle;
    surface.repeat = state.repeat;
    // Deliberately untouched: eq_selected (the 127/128 latch is local to one
    // plugin's query sequence), visible_*, *_hwnd, menu_handle and
    // inet_available (no protocol message carries them; see module docs).
}

/// Apply one inbound protocol message to the surface.
///
/// Returns `true` iff the message carried player state (currently only
/// [`Message::PlayerStateUpdate`]). Every other variant returns `false` and
/// leaves the surface untouched -- that is the honest "unknown case", not a
/// silent default.
pub fn apply_message(surface: &mut IpcSurface, message: &Message) -> bool {
    match message {
        Message::PlayerStateUpdate(state) => {
            apply_player_state(surface, state);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_proto::state::{EqState, StreamInfo};

    fn playing_state() -> PlayerState {
        PlayerState {
            transport: TransportState::Playing,
            position_ms: 12_345,
            duration_ms: Some(200_000),
            volume: 200,
            balance: -10,
            shuffle: true,
            repeat: true,
            current_index: Some(7),
            current_id: None,
            playlist_len: 60,
            stream: Some(StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: 44_100,
                channels: 2,
            }),
            eq: EqState {
                bands: [12.0, -12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                preamp: 0.0,
                enabled: true,
                auto: false,
            },
            stop_after_current: false,
        }
    }

    #[test]
    fn eq_anchors_hit_s09_exactly_and_saturate() {
        assert_eq!(eq_classic_from_db(12.0), 0);
        assert_eq!(eq_classic_from_db(0.0), 31);
        assert_eq!(eq_classic_from_db(-12.0), 63);
        assert_eq!(eq_classic_from_db(24.0), 0);
        assert_eq!(eq_classic_from_db(-24.0), 63);
        assert_eq!(eq_classic_from_db(f32::NAN), 31);
    }

    #[test]
    fn transport_maps_to_0_1_3() {
        assert_eq!(classic_playing_value(TransportState::Stopped), 0);
        assert_eq!(classic_playing_value(TransportState::Playing), 1);
        assert_eq!(classic_playing_value(TransportState::Paused), 3);
    }

    #[test]
    fn applied_state_answers_through_the_measured_ids() {
        let mut surface = IpcSurface::new();
        apply_player_state(&mut surface, &playing_state());
        // Id 104 IPC_ISPLAYING.
        assert_eq!(surface.answer(104, 0).raw(), 1);
        // Id 105 IPC_GETOUTPUTTIME modes 0/1/2.
        assert_eq!(surface.answer(105, 0).raw(), 12_345);
        assert_eq!(surface.answer(105, 1).raw(), 200);
        assert_eq!(surface.answer(105, 2).raw(), 200_000);
        // Id 122 volume query, id 123 pan query (signed -10).
        assert_eq!(surface.answer(122, -666).raw(), 200);
        assert_eq!(surface.answer(123, -666).raw(), (-10_i32) as u32);
        // Ids 124/125 playlist length/position.
        assert_eq!(surface.answer(124, 0).raw(), 60);
        assert_eq!(surface.answer(125, 0).raw(), 7);
        // Id 126 IPC_GETINFO modes 0/1/2.
        assert_eq!(surface.answer(126, 0).raw(), 44_100);
        assert_eq!(surface.answer(126, 1).raw(), 128);
        assert_eq!(surface.answer(126, 2).raw(), 2);
        // Id 127 equalizer: +12 dB -> 0, -12 dB -> 63, 0 dB -> 31.
        assert_eq!(surface.answer(127, 0).raw(), 0);
        assert_eq!(surface.answer(127, 1).raw(), 63);
        assert_eq!(surface.answer(127, 2).raw(), 31);
        assert_eq!(surface.answer(127, 10).raw(), 31);
        assert_eq!(surface.answer(127, 11).raw(), 1);
        assert_eq!(surface.answer(127, 12).raw(), 0);
        // Ids 250/251 shuffle/repeat.
        assert_eq!(surface.answer(250, 0).raw(), 1);
        assert_eq!(surface.answer(251, 0).raw(), 1);
    }

    #[test]
    fn paused_transport_is_visible_and_stopped_keeps_stopped_answers() {
        let mut surface = IpcSurface::new();
        let mut paused = playing_state();
        paused.transport = TransportState::Paused;
        apply_player_state(&mut surface, &paused);
        assert_eq!(surface.answer(104, 0).raw(), 3);

        let stopped = PlayerState::default();
        apply_player_state(&mut surface, &stopped);
        assert_eq!(surface.answer(104, 0).raw(), 0);
        // While stopped the surface keeps the documented stopped answers:
        // 105 mode 0/1 -> -1, mode 2 -> 0; 126 -> 0.
        assert_eq!(surface.answer(105, 0).raw(), 4_294_967_295);
        assert_eq!(surface.answer(105, 2).raw(), 0);
        assert_eq!(surface.answer(126, 0).raw(), 0);
        // Empty optionals fall back to the fresh defaults.
        assert_eq!(surface.answer(124, 0).raw(), 0);
        assert_eq!(surface.answer(125, 0).raw(), 0);
    }

    #[test]
    fn only_player_state_update_applies_and_others_leave_the_surface() {
        let mut surface = IpcSurface::new();
        assert!(apply_message(
            &mut surface,
            &Message::PlayerStateUpdate(Box::new(playing_state()))
        ));
        assert_eq!(surface.answer(104, 0).raw(), 1);

        for other in [
            Message::Ping(7),
            Message::Shutdown,
            Message::DecoderEos,
            Message::WinampIpc {
                msg_id: 104,
                wparam: 0,
                lparam: 0,
            },
        ] {
            assert!(!apply_message(&mut surface, &other));
        }
        // The rejected messages changed nothing.
        assert_eq!(surface.answer(104, 0).raw(), 1);
        assert_eq!(surface.answer(124, 0).raw(), 60);
    }

    #[test]
    fn bridge_does_not_touch_host_owned_fields() {
        let mut surface = IpcSurface::new();
        surface.main_hwnd = 0x1234;
        surface.menu_handle = 0x5678;
        surface.visible_eq = false;
        surface.inet_available = false;
        apply_player_state(&mut surface, &playing_state());
        assert_eq!(surface.main_hwnd, 0x1234);
        assert_eq!(surface.menu_handle, 0x5678);
        assert!(!surface.visible_eq);
        assert!(!surface.inet_available);
        assert!(surface.eq_selected.is_none());
    }
}
