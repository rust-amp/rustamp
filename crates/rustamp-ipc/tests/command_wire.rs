//! The RFC 0005 wire proof: every pre-0005 [`Command`] still encodes to exactly
//! the bytes it encoded to before the app-level variants were appended, and the
//! new variants sit behind them.
//!
//! `postcard` encodes an enum by its **declaration index** and a struct
//! positionally, so "appended" is a byte-level claim, not a style preference.
//! The expected byte strings below are written out by hand from the pre-0005
//! definition (variant order and payload shapes) and the encoding rules
//! `postcard` applies — unsigned integers and string/vector lengths as
//! LEB128 varints, `u8`/`i8` as one raw byte, signed integers zigzagged, `f32`
//! as 4 little-endian bytes, `bool` as one byte and `Option` as a one-byte tag.
//! They are not produced by the code under test.

use rustamp_proto::command::{Command, JumpTarget, OpenMediaKind, PrefsPage, SelectOp};
use rustamp_proto::playlist::{EntryId, MediaLocation, SortKey};
use rustamp_proto::state::{PlayerState, WindowId};

fn encode(command: &Command) -> Vec<u8> {
    postcard::to_allocvec(command).expect("encoding a command")
}

fn decode(bytes: &[u8]) -> Command {
    postcard::from_bytes(bytes).expect("decoding a command")
}

/// One pre-0005 command and the bytes it had before RFC 0005.
struct Vector {
    name: &'static str,
    command: Command,
    bytes: &'static [u8],
}

/// Every one of the 36 pre-0005 variants, in declaration order.
fn pre_0005_vectors() -> Vec<Vector> {
    let mut v = Vec::new();
    let mut push = |name: &'static str, command: Command, bytes: &'static [u8]| {
        v.push(Vector {
            name,
            command,
            bytes,
        })
    };

    push("Play", Command::Play, &[0x00]);
    push("Pause", Command::Pause, &[0x01]);
    push("TogglePause", Command::TogglePause, &[0x02]);
    push("Stop", Command::Stop, &[0x03]);
    push("StopWithFadeout", Command::StopWithFadeout, &[0x04]);
    push("Next", Command::Next, &[0x05]);
    push("Prev", Command::Prev, &[0x06]);
    push("Seek", Command::Seek { to_ms: 1500 }, &[0x07, 0xdc, 0x0b]);
    push(
        "SeekRelative back",
        Command::SeekRelative { delta_ms: -5000 },
        &[0x08, 0x8f, 0x4e],
    );
    push(
        "SeekRelative forward",
        Command::SeekRelative { delta_ms: 5000 },
        &[0x08, 0x90, 0x4e],
    );
    push("SetVolume(128)", Command::SetVolume(128), &[0x09, 0x80]);
    push("SetVolume(255)", Command::SetVolume(255), &[0x09, 0xff]);
    push("SetVolume(4)", Command::SetVolume(4), &[0x09, 0x04]);
    push("SetBalance(-127)", Command::SetBalance(-127), &[0x0a, 0x81]);
    push("SetBalance(0)", Command::SetBalance(0), &[0x0a, 0x00]);
    push("ToggleShuffle", Command::ToggleShuffle, &[0x0b]);
    push("ToggleRepeat", Command::ToggleRepeat, &[0x0c]);
    push("SetShuffle(true)", Command::SetShuffle(true), &[0x0d, 0x01]);
    push("SetRepeat(false)", Command::SetRepeat(false), &[0x0e, 0x00]);
    push(
        "AddItems",
        Command::AddItems {
            items: vec![
                MediaLocation::File("C:\\m\\a.mp3".into()),
                MediaLocation::Url("http://h/s".into()),
            ],
            at: Some(3),
            play: true,
        },
        &[
            0x0f, 0x02, // two items
            0x00, 0x0a, 0x43, 0x3a, 0x5c, 0x6d, 0x5c, 0x61, 0x2e, 0x6d, 0x70, 0x33, // File
            0x01, 0x0a, 0x68, 0x74, 0x74, 0x70, 0x3a, 0x2f, 0x2f, 0x68, 0x2f, 0x73, // Url
            0x01, 0x03, // at = Some(3)
            0x01, // play
        ],
    );
    push(
        "Remove",
        Command::Remove(vec![EntryId(1), EntryId(2)]),
        &[0x10, 0x02, 0x01, 0x02],
    );
    push("Clear", Command::Clear, &[0x11]);
    push(
        "MoveEntries",
        Command::MoveEntries {
            ids: vec![EntryId(7)],
            to: 2,
        },
        &[0x12, 0x01, 0x07, 0x02],
    );
    push("Select(All)", Command::Select(SelectOp::All), &[0x13, 0x02]);
    push(
        "Select(Set)",
        Command::Select(SelectOp::Set(vec![EntryId(5)])),
        &[0x13, 0x00, 0x01, 0x05],
    );
    push("PlayIndex(0)", Command::PlayIndex(0), &[0x14, 0x00]);
    push("PlayIndex(7)", Command::PlayIndex(7), &[0x14, 0x07]);
    push(
        "PlayEntry(1)",
        Command::PlayEntry(EntryId(1)),
        &[0x15, 0x01],
    );
    push(
        "Sort(PathAndFileName)",
        Command::Sort(SortKey::PathAndFileName),
        &[0x16, 0x02],
    );
    push(
        "Sort(Duration)",
        Command::Sort(SortKey::Duration),
        &[0x16, 0x03],
    );
    push("Randomize", Command::Randomize, &[0x17]);
    push("Reverse", Command::Reverse, &[0x18]);
    push(
        "SetBand",
        Command::SetBand {
            band: 3,
            gain_db: -6.0,
        },
        &[0x19, 0x03, 0x00, 0x00, 0xc0, 0xc0],
    );
    push(
        "SetPreamp",
        Command::SetPreamp(2.5),
        &[0x1a, 0x00, 0x00, 0x20, 0x40],
    );
    push("SetEqEnabled", Command::SetEqEnabled(true), &[0x1b, 0x01]);
    push("SetEqAuto", Command::SetEqAuto(false), &[0x1c, 0x00]);
    push(
        "ToggleWindow(Vis)",
        Command::ToggleWindow(WindowId::Vis),
        &[0x1d, 0x03],
    );
    push(
        "ToggleWindow(Gen)",
        Command::ToggleWindow(WindowId::Gen(9)),
        &[0x1d, 0x04, 0x09],
    );
    push("ToggleDoubleSize", Command::ToggleDoubleSize, &[0x1e]);
    push(
        "ToggleWindowShade(Gen)",
        Command::ToggleWindowShade(WindowId::Gen(9)),
        &[0x1f, 0x04, 0x09],
    );
    push("ToggleAlwaysOnTop", Command::ToggleAlwaysOnTop, &[0x20]);
    push(
        "LoadSkin",
        Command::LoadSkin("C:\\s\\x.wsz".into()),
        &[
            0x21, 0x0a, 0x43, 0x3a, 0x5c, 0x73, 0x5c, 0x78, 0x2e, 0x77, 0x73, 0x7a,
        ],
    );
    push("Quit", Command::Quit, &[0x22]);
    push("OpenPrefs", Command::OpenPrefs, &[0x23]);

    v
}

#[test]
fn every_pre_0005_command_encodes_to_its_pre_0005_bytes() {
    let vectors = pre_0005_vectors();
    // 44 payload variants, and — because the first byte is the variant index —
    // exactly the 36 pre-0005 variants, `Play` (0) through `OpenPrefs` (35).
    assert_eq!(vectors.len(), 44);
    let indices: std::collections::BTreeSet<u8> =
        vectors.iter().map(|vector| vector.bytes[0]).collect();
    assert_eq!(indices.len(), 36, "one vector per pre-0005 variant");
    assert_eq!(indices.iter().next(), Some(&0x00));
    assert_eq!(indices.iter().next_back(), Some(&0x23));
    for Vector {
        name,
        command,
        bytes,
    } in vectors
    {
        assert_eq!(encode(&command), bytes, "{name}");
    }
}

#[test]
fn every_pre_0005_byte_string_still_decodes_to_the_same_command() {
    // Append-only keeps old -> new decoding intact: a v2 peer decodes every v1
    // command unchanged (RFC 0005 §12).
    for Vector {
        name,
        command,
        bytes,
    } in pre_0005_vectors()
    {
        assert_eq!(decode(bytes), command, "{name}");
    }
}

#[test]
fn the_new_variants_start_where_open_prefs_ends() {
    // Appended after `OpenPrefs` (index 35), so the new block starts at 36.
    // The first column is the variant's declaration index; a variant with two
    // payloads (`SkipBy`) appears twice with the same index.
    let new_variants: [(u32, Command, &[u8]); 20] = [
        (36, Command::SkipBy { delta: 10 }, &[0x24, 0x14]),
        (36, Command::SkipBy { delta: -10 }, &[0x24, 0x13]),
        (37, Command::PlayLast, &[0x25]),
        (38, Command::ToggleStopAfterCurrent, &[0x26]),
        (
            39,
            Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            },
            &[0x27, 0x00, 0x01],
        ),
        (
            39,
            Command::OpenMedia {
                kind: OpenMediaKind::Location,
                play: false,
            },
            &[0x27, 0x01, 0x00],
        ),
        (
            39,
            Command::OpenMedia {
                kind: OpenMediaKind::Directory,
                play: true,
            },
            &[0x27, 0x02, 0x01],
        ),
        (
            40,
            Command::OpenJump {
                target: JumpTarget::File,
            },
            &[0x28, 0x00],
        ),
        (
            40,
            Command::OpenJump {
                target: JumpTarget::Time,
            },
            &[0x28, 0x01],
        ),
        (41, Command::OpenFileInfo, &[0x29]),
        (
            42,
            Command::OpenPrefsPage(PrefsPage::VisPlugin),
            &[0x2a, 0x00],
        ),
        (
            42,
            Command::OpenPrefsPage(PrefsPage::VisOptions),
            &[0x2a, 0x01],
        ),
        (43, Command::OpenAbout, &[0x2b]),
        (44, Command::OpenSkinBrowser, &[0x2c]),
        (45, Command::RefreshSkin, &[0x2d]),
        (46, Command::OpenBookmarkEditor, &[0x2e]),
        (47, Command::AddBookmark, &[0x2f]),
        (48, Command::ToggleEasyMove, &[0x30]),
        (49, Command::Minimize, &[0x31]),
        (50, Command::FocusNextWindow, &[0x32]),
    ];
    for (index, command, bytes) in new_variants {
        assert_eq!(encode(&command), bytes, "index {index}");
    }
}

#[test]
fn the_last_five_new_variants_and_the_video_window() {
    // The rest of the appended block (51..=55) and `WindowId::Video`, which is
    // appended after `Gen` so that `Gen`'s index — and every pre-0005 byte
    // string that carries it — is unchanged.
    assert_eq!(encode(&Command::ShowMainWindow), [0x33]);
    assert_eq!(encode(&Command::CloseFocusedWindow), [0x34]);
    assert_eq!(encode(&Command::OpenMainMenu), [0x35]);
    assert_eq!(encode(&Command::ConfigureVisPlugin), [0x36]);
    assert_eq!(encode(&Command::ToggleVisPlugin), [0x37]);
    assert_eq!(
        encode(&Command::ToggleWindow(WindowId::Video)),
        [0x1d, 0x05]
    );
    assert_eq!(
        encode(&Command::ToggleWindowShade(WindowId::Video)),
        [0x1f, 0x05]
    );
}

#[test]
fn the_clutter_menu_commands_are_appended_after_open_playlist() {
    // RFC 0016: `OpenPlaylist` is index 56 (0x38); the two clutter-bar menu
    // commands follow it, so no earlier byte string moves.
    assert_eq!(encode(&Command::OpenPlaylist), [0x38]);
    assert_eq!(encode(&Command::OpenOptionsMenu), [0x39]);
    assert_eq!(encode(&Command::OpenVisualizationMenu), [0x3a]);
    for command in [Command::OpenOptionsMenu, Command::OpenVisualizationMenu] {
        assert_eq!(decode(&encode(&command)), command);
    }
}

#[test]
fn save_playlist_is_appended_after_the_clutter_menus() {
    // RFC 0017: `SavePlaylist` is index 59 (0x3b), after the clutter-bar menu
    // commands (0x39, 0x3a), so no earlier byte string moves.
    let command = Command::SavePlaylist("list.m3u".into());
    let bytes = encode(&command);
    assert_eq!(bytes[0], 0x3b);
    assert_eq!(decode(&bytes), command);
}

#[test]
fn remove_dead_files_is_appended_after_save_playlist() {
    // RFC 0018: `RemoveDeadFiles` is index 60 (0x3c), after `SavePlaylist`
    // (0x3b), so no earlier byte string moves.
    let command = Command::RemoveDeadFiles;
    let bytes = encode(&command);
    assert_eq!(bytes, [0x3c]);
    assert_eq!(decode(&bytes), command);
}

#[test]
fn every_new_variant_round_trips() {
    let commands = [
        Command::SkipBy { delta: -3 },
        Command::PlayLast,
        Command::ToggleStopAfterCurrent,
        Command::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: false,
        },
        Command::OpenJump {
            target: JumpTarget::Time,
        },
        Command::OpenFileInfo,
        Command::OpenPrefsPage(PrefsPage::VisOptions),
        Command::OpenAbout,
        Command::OpenSkinBrowser,
        Command::RefreshSkin,
        Command::OpenBookmarkEditor,
        Command::AddBookmark,
        Command::ToggleEasyMove,
        Command::Minimize,
        Command::FocusNextWindow,
        Command::ShowMainWindow,
        Command::CloseFocusedWindow,
        Command::OpenMainMenu,
        Command::ConfigureVisPlugin,
        Command::ToggleVisPlugin,
    ];
    assert_eq!(commands.len(), 20, "the RFC 0005 block has 20 variants");
    for command in commands {
        assert_eq!(decode(&encode(&command)), command);
    }
}

#[test]
fn command_to_host_is_appended_to_the_end_of_message() {
    use rustamp_ipc::Message;

    // The `Message` indices the pre-0005 protocol pinned: 7 is
    // `CommandFromPlugin`, 12 `Shutdown`, 13 `WinampIpc`.
    assert_eq!(
        postcard::to_allocvec(&Message::CommandFromPlugin(Command::Play)).unwrap(),
        [0x07, 0x00]
    );
    assert_eq!(postcard::to_allocvec(&Message::Shutdown).unwrap(), [0x0c]);
    assert_eq!(
        postcard::to_allocvec(&Message::WinampIpc {
            msg_id: 1,
            wparam: 0,
            lparam: 0
        })
        .unwrap(),
        [0x0d, 0x01, 0x00, 0x00]
    );
    // The new variant is the next index (14), not an inserted one.
    let bytes = postcard::to_allocvec(&Message::CommandToHost(Command::Play)).unwrap();
    assert_eq!(bytes, [0x0e, 0x00]);
    assert_eq!(
        postcard::from_bytes::<Message>(&bytes).unwrap(),
        Message::CommandToHost(Command::Play)
    );
}

#[test]
fn the_protocol_version_is_four() {
    // RFC 0005 took the protocol to 2; RFC 0006 appended its hosting messages
    // and took it to 3; RFC 0015's DspResetAck takes it to 4, because a v3 peer
    // would mis-decode the new variant (the project rule is a clean Hello
    // rejection rather than a decode error).
    assert_eq!(rustamp_ipc::PROTOCOL_VERSION, 4);
    assert_eq!(
        rustamp_ipc::Message::hello(),
        rustamp_ipc::Message::Hello {
            protocol_version: 4,
            pid: std::process::id(),
        }
    );
}

#[test]
fn player_state_is_positional_with_the_new_field_last() {
    // Hand-derived from the definition: the transport enum index, then the
    // fields in declaration order — `position_ms` (varint), `duration_ms`
    // (Option tag), `volume` (a raw byte, default 255), `balance`, `shuffle`,
    // `repeat`, `current_index` (Option tag), `current_id` (Option tag),
    // `playlist_len` (varint), `stream` (Option tag), the 10 band floats +
    // preamp + enabled + auto of `EqState` (46 bytes) and finally the RFC 0005
    // `stop_after_current` byte. The defaults are all zero except the volume.
    let mut expected = vec![0u8; 58];
    expected[3] = 0xff;
    assert_eq!(
        postcard::to_allocvec(&PlayerState::default()).unwrap(),
        expected
    );

    // The new field is the last byte, so every pre-0005 field keeps its offset.
    let armed = PlayerState {
        stop_after_current: true,
        ..PlayerState::default()
    };
    let mut expected_armed = expected.clone();
    expected_armed[57] = 0x01;
    assert_eq!(
        postcard::to_allocvec(&armed).unwrap(),
        expected_armed,
        "only the last byte may differ"
    );
    assert_eq!(
        postcard::from_bytes::<PlayerState>(&expected_armed).unwrap(),
        armed
    );
}
