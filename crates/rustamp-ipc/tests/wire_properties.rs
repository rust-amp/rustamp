//! Seeded property tests for the IPC wire itself.
//!
//! The hand-written vectors in `command_wire.rs` and `hosting_wire.rs` pin
//! the bytes of a few examples. These tests cover the whole surface: every
//! [`Message`] variant and every [`Command`] variant, with random field
//! values, over a seeded generator (this file's own SplitMix64 - no new
//! dependency, and the published crates keep their dependency list and its
//! license closure untouched). Every case is reproducible from the seed the
//! failure message prints.
//!
//! The properties are:
//!
//! 1. **Every variant survives a round trip.** `decode(encode(x)) == x` for
//!    every `Message` variant, and for every `Command` variant both on its
//!    own and inside `Message::CommandToHost` (the direction the app uses).
//! 2. **The declaration indices are the documented ones.** `postcard` encodes
//!    an enum by its declaration index, so the index of a variant is part of
//!    the wire. The contract table in `support/wire_gen.rs` names the index
//!    of every variant; a variant inserted into the middle of an enum moves
//!    every later index and fails here.
//! 3. **A truncated frame is refused.** Every strict prefix of an encoded
//!    message either fails to decode or decodes to something else - never to
//!    the original message. The framing refuses a cut stream as well.
//!    Measured over the whole corpus: **no** strict prefix decodes at all -
//!    `postcard` requires every field and no field has a default, so there is
//!    no trailing-optional field that could let a prefix through. The
//!    assertion below is the weaker contract, and it stays true if the codec
//!    ever grows defaults.
//! 4. **A corrupted byte is refused or different.** Flipping one bit must
//!    never yield the original message again.
//!
//! The corruption property is checked at the **byte** level, because that is
//! the level the wire lives at: `f32`'s `PartialEq` equates `+0.0` and
//! `-0.0`, so a sign-bit flip yields a message that *compares equal* to the
//! original while its bytes differ. The test
//! `a_signed_zero_corrupts_the_bytes_but_not_the_number` pins that exception.
//!
//! 5. **The protocol version agrees with its documented history.** The last
//!    entry of the version history in `message.rs` names the current
//!    [`PROTOCOL_VERSION`](rustamp_ipc::PROTOCOL_VERSION).

#[path = "support/wire_gen.rs"]
mod wire_gen;

use std::io::Cursor;

use rustamp_ipc::{IpcError, Message, decode, encode, read_frame, write_frame};
use rustamp_proto::command::Command;

use crate::wire_gen::{
    COMMAND_VARIANT_COUNT, MESSAGE_VARIANT_COUNT, Rng, command_variant, message_variant,
};

/// Seed of the random message and command streams.
const SEED: u64 = 0x5EED_2960_0000_0001;

/// Number of random messages per variant in the round-trip property.
const SAMPLES: usize = 24;

/// Number of random messages per variant in the byte-level properties.
///
/// The corruption property flips *every* bit of every one of them, so fewer
/// samples keep the run short.
const BYTE_SAMPLES: usize = 4;

/// The seed of one variant's stream: reproducible from the seed alone.
fn variant_seed(index: u8) -> u64 {
    SEED.wrapping_add(u64::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Encode a message.
fn encode_message(message: &Message) -> Vec<u8> {
    encode(message).expect("encoding a message")
}

/// Encode a command.
fn encode_command(command: &Command) -> Vec<u8> {
    postcard::to_allocvec(command).expect("encoding a command")
}

/// Decode a command.
fn decode_command(bytes: &[u8]) -> Result<Command, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[test]
fn every_message_variant_survives_a_round_trip() {
    for index in 0..MESSAGE_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, _) = message_variant(index, &mut rng).expect("a documented Message variant");
        for _ in 0..SAMPLES {
            let (variant_name, message) =
                message_variant(index, &mut rng).expect("a documented Message variant");
            assert_eq!(
                variant_name, name,
                "the name of a variant must not depend on its random values"
            );
            let bytes = encode_message(&message);
            let decoded = decode(&bytes).unwrap_or_else(|error| {
                panic!("{name}: decoding {bytes:02x?} failed: {error} (seed {SEED:#x})")
            });
            assert_eq!(
                decoded, message,
                "{name}: the round trip changed the message (seed {SEED:#x})"
            );
        }
    }
}

#[test]
fn every_command_variant_survives_a_round_trip() {
    for index in 0..COMMAND_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, _) = command_variant(index, &mut rng).expect("a documented Command variant");
        for _ in 0..SAMPLES {
            let (variant_name, command) =
                command_variant(index, &mut rng).expect("a documented Command variant");
            assert_eq!(
                variant_name, name,
                "the name of a variant must not depend on its random values"
            );

            // On its own.
            let bytes = encode_command(&command);
            let decoded = decode_command(&bytes).unwrap_or_else(|error| {
                panic!("{name}: decoding {bytes:02x?} failed: {error} (seed {SEED:#x})")
            });
            assert_eq!(
                decoded, command,
                "{name}: the round trip changed the command (seed {SEED:#x})"
            );

            // Inside the message the app sends to the host.
            let message = Message::CommandToHost(command.clone());
            let bytes = encode_message(&message);
            let decoded = decode(&bytes).unwrap_or_else(|error| {
                panic!("{name}: decoding {bytes:02x?} failed: {error} (seed {SEED:#x})")
            });
            assert_eq!(
                decoded, message,
                "{name}: the round trip changed the message (seed {SEED:#x})"
            );
        }
    }
}

#[test]
fn every_command_variant_survives_the_plugin_direction_too() {
    // The other direction carries the same `Command`: a plugin asks the
    // player to do something. The command's bytes do not depend on the
    // wrapper - only the message's variant index does.
    for index in 0..COMMAND_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, command) =
            command_variant(index, &mut rng).expect("a documented Command variant");
        let message = Message::CommandFromPlugin(command.clone());
        let bytes = encode_message(&message);
        let decoded = decode(&bytes).unwrap_or_else(|error| {
            panic!("{name}: decoding {bytes:02x?} failed: {error} (seed {SEED:#x})")
        });
        assert_eq!(
            decoded, message,
            "{name}: the round trip changed the message (seed {SEED:#x})"
        );

        // The payload is the command's own bytes; the wrapper only adds the
        // variant index in front of it.
        let to_host = encode_message(&Message::CommandToHost(command));
        assert_eq!(
            &bytes[1..],
            &to_host[1..],
            "{name}: the command must encode the same in both directions"
        );
        assert_eq!(bytes[0], 7, "{name}: CommandFromPlugin is variant 7");
        assert_eq!(to_host[0], 14, "{name}: CommandToHost is variant 14");
    }
}

#[test]
fn every_variant_keeps_its_documented_declaration_index() {
    for index in 0..MESSAGE_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, message) =
            message_variant(index, &mut rng).expect("a documented Message variant");
        let bytes = encode_message(&message);
        assert_eq!(
            bytes[0], index,
            "{name}: the encoded variant index moved (seed {SEED:#x})"
        );
    }
    for index in 0..COMMAND_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, command) =
            command_variant(index, &mut rng).expect("a documented Command variant");
        let bytes = encode_command(&command);
        assert_eq!(
            bytes[0], index,
            "{name}: the encoded variant index moved (seed {SEED:#x})"
        );
    }
}

#[test]
fn the_contract_table_covers_every_variant() {
    // Every index below the count names a variant ...
    for index in 0..MESSAGE_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        assert!(
            message_variant(index, &mut rng).is_some(),
            "the Message variant {index} is missing from the contract table"
        );
    }
    for index in 0..COMMAND_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        assert!(
            command_variant(index, &mut rng).is_some(),
            "the Command variant {index} is missing from the contract table"
        );
    }
    // ... and the first index past the count names none. `postcard` refuses an
    // unknown variant index, so a variant that was added to an enum without a
    // table entry fails here instead of going unnoticed.
    //
    // The payload behind the index is a run of zeros: long enough for any
    // variant shape (a unit variant needs nothing, a `u64` a varint, a
    // `String` a length, a nested message its own bytes), so an index that
    // *is* a variant decodes and the check fails - while an index that is not
    // a variant is refused for the right reason.
    let mut past = vec![MESSAGE_VARIANT_COUNT];
    past.extend_from_slice(&[0u8; 16]);
    assert!(
        postcard::from_bytes::<Message>(&past).is_err(),
        "the Message enum has a variant the contract table does not list"
    );
    // `Message::CommandFromPlugin` is the variant that carries a `Command`
    // (declaration index 7).
    let mut past = vec![7, COMMAND_VARIANT_COUNT];
    past.extend_from_slice(&[0u8; 16]);
    assert!(
        postcard::from_bytes::<Message>(&past).is_err(),
        "the Command enum has a variant the contract table does not list"
    );
}

#[test]
fn a_truncated_payload_is_never_decoded_as_the_whole_message() {
    for index in 0..MESSAGE_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, _) = message_variant(index, &mut rng).expect("a documented Message variant");
        for _ in 0..SAMPLES {
            let (_, message) =
                message_variant(index, &mut rng).expect("a documented Message variant");
            let bytes = encode_message(&message);
            for cut in 0..bytes.len() {
                let prefix = &bytes[..cut];
                if let Ok(decoded) = decode(prefix) {
                    assert_ne!(
                        encode_message(&decoded),
                        bytes,
                        "{name}: a payload cut to {cut} of {} bytes decoded to the original message \
                         (seed {SEED:#x})",
                        bytes.len()
                    );
                }
            }
        }
    }
}

#[test]
fn a_truncated_frame_is_refused_by_the_framing() {
    let mut rng = Rng::new(variant_seed(9));
    let (name, message) = message_variant(9, &mut rng).expect("a documented Message variant");
    let mut frame = Vec::new();
    write_frame(&mut frame, &message).expect("writing a frame");

    // The whole frame is the message again.
    let mut cursor = Cursor::new(&frame);
    assert_eq!(
        read_frame(&mut cursor).expect("reading a whole frame"),
        message,
        "{name}: the whole frame must decode"
    );

    // Every cut of the stream is an error, never a message.
    for cut in 0..frame.len() {
        let mut cursor = Cursor::new(&frame[..cut]);
        assert!(
            read_frame(&mut cursor).is_err(),
            "{name}: a frame cut to {cut} of {} bytes was accepted (seed {SEED:#x})",
            frame.len()
        );
    }
}

#[test]
fn a_corrupted_payload_is_never_accepted_as_the_original_message() {
    for index in 0..MESSAGE_VARIANT_COUNT {
        let mut rng = Rng::new(variant_seed(index));
        let (name, _) = message_variant(index, &mut rng).expect("a documented Message variant");
        for _ in 0..BYTE_SAMPLES {
            let (_, message) =
                message_variant(index, &mut rng).expect("a documented Message variant");
            let bytes = encode_message(&message);
            for bit in 0..bytes.len() * 8 {
                let mut corrupted = bytes.clone();
                corrupted[bit / 8] ^= 1 << (bit % 8);
                if let Ok(decoded) = decode(&corrupted) {
                    assert_ne!(
                        encode_message(&decoded),
                        bytes,
                        "{name}: bit {bit} of a corrupted payload decoded to the original message \
                         (seed {SEED:#x})",
                    );
                }
            }
        }
    }
}

#[test]
fn a_corrupted_frame_is_refused_or_different_in_the_framing() {
    let mut rng = Rng::new(variant_seed(9));
    let (name, message) = message_variant(9, &mut rng).expect("a documented Message variant");
    let mut frame = Vec::new();
    write_frame(&mut frame, &message).expect("writing a frame");
    // The payload without the length prefix.
    let payload = &frame[4..];

    for bit in 0..frame.len() * 8 {
        let mut corrupted = frame.clone();
        corrupted[bit / 8] ^= 1 << (bit % 8);
        let mut cursor = Cursor::new(&corrupted);
        if let Ok(decoded) = read_frame(&mut cursor) {
            // A frame that decodes at all was framed with an intact length
            // prefix (a corrupted one makes the frame too short or too long),
            // so the payload is compared byte for byte.
            assert_ne!(
                encode_message(&decoded),
                payload,
                "{name}: bit {bit} of a corrupted frame decoded to the original message \
                 (seed {SEED:#x})",
            );
        }
    }
}

#[test]
fn a_signed_zero_corrupts_the_bytes_but_not_the_number() {
    // `f32`'s `PartialEq` equates `+0.0` and `-0.0`. A sign-bit flip
    // therefore yields a command that *compares equal* to the original while
    // its bytes differ: the decoder does not confuse the two - it hands back
    // exactly the bytes it received - but a value comparison cannot tell them
    // apart. The corruption property is therefore checked at the byte level,
    // and this test pins the exception so nobody mistakes it for a broken
    // decoder later.
    let command = Command::SetPreamp(0.0);
    let bytes = encode_command(&command);
    assert_eq!(bytes, [0x1a, 0x00, 0x00, 0x00, 0x00]);

    let mut corrupted = bytes.clone();
    corrupted[4] ^= 0x80; // the sign bit of the float
    let decoded = decode_command(&corrupted).expect("a signed zero is a valid float");
    assert_eq!(decoded, command, "+0.0 and -0.0 are the same number");
    assert_ne!(
        encode_command(&decoded),
        bytes,
        "but they are not the same frame"
    );
}

/// The protocol versions the `PROTOCOL_VERSION` doc comment lists.
///
/// The history is prose, so it is read out of the source: the bullet list
/// above the constant. A version bump that forgets the history fails here.
fn documented_protocol_versions() -> Vec<u32> {
    let source = include_str!("../src/message.rs");
    let lines: Vec<&str> = source.lines().collect();
    let declaration = lines
        .iter()
        .position(|line| line.trim_start().starts_with("pub const PROTOCOL_VERSION"))
        .expect("the PROTOCOL_VERSION constant");
    let mut doc: Vec<&str> = Vec::new();
    for line in lines[..declaration].iter().rev() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("///") {
            doc.push(trimmed);
        } else {
            break;
        }
    }
    doc.reverse();
    doc.iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("/// * `")?;
            let end = rest.find('`').expect("a documented version number");
            Some(rest[..end].parse().expect("a documented version number"))
        })
        .collect()
}

#[test]
fn the_protocol_version_agrees_with_its_documented_history() {
    let history = documented_protocol_versions();
    assert!(
        !history.is_empty(),
        "the protocol version must document its history"
    );
    assert_eq!(
        history[0], 0,
        "the documented history must start at version 0"
    );
    for (index, version) in history.iter().enumerate() {
        assert_eq!(
            *version as usize, index,
            "the documented versions must be contiguous: {history:?}"
        );
    }
    assert_eq!(
        history.last().copied(),
        Some(rustamp_ipc::PROTOCOL_VERSION),
        "the last documented version must be the current one: {history:?}"
    );
}

#[test]
fn the_decoder_reports_a_codec_error_for_a_broken_payload() {
    // The properties above only care that a broken payload does not decode
    // into the original message; this pins the error kind, so a caller can
    // tell a codec error from a closed connection.
    let error: IpcError = decode(&[0xff]).expect_err("an unknown variant index must fail");
    assert!(
        matches!(error, IpcError::Codec(_)),
        "an unknown variant index must be a codec error, got {error:?}"
    );
}
