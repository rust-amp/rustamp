//! The RFC 0006 wire proof: the new hosting [`Message`] variants encode and
//! decode correctly, and their variant indices are appended after the RFC 0005
//! block (which ended at `CommandToHost`, index 14). RFC 0015's `DspResetAck`
//! rides in the same DSP block, beside `DspReset`.
//!
//! `postcard` encodes an enum by its **declaration index** and a struct
//! positionally, so "appended" is a byte-level claim.

use rustamp_ipc::Message;
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};
use rustamp_proto::meta::TrackMetadata;

fn encode(msg: &Message) -> Vec<u8> {
    postcard::to_allocvec(msg).expect("encoding a message")
}

fn decode(bytes: &[u8]) -> Message {
    postcard::from_bytes(bytes).expect("decoding a message")
}

/// The pre-0006 protocol had 15 variants (0..=14), with `CommandToHost` at index 14.
/// The hosting block is RFC 0006's 12 variants (indices 15..=26) plus RFC
/// 0015's `DspResetAck` (index 26), which pushed `DspError` to 27.
const HOSTING_VARIANT_COUNT: usize = 13;

#[test]
fn protocol_version_is_seven() {
    assert_eq!(rustamp_ipc::PROTOCOL_VERSION, 7);
    assert_eq!(
        rustamp_ipc::Message::hello(),
        rustamp_ipc::Message::Hello {
            protocol_version: 7,
            pid: std::process::id(),
        }
    );
}

#[test]
fn decoder_open_roundtrip() {
    let msg = Message::DecoderOpen {
        location: MediaLocationWire::File("C:\\Music\\track.mp3".into()),
        prefetch_frames: 4096,
    };
    let bytes = encode(&msg);
    // Variant index 15 = 0x0f
    assert_eq!(bytes[0], 0x0f);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_open_url_roundtrip() {
    let msg = Message::DecoderOpen {
        location: MediaLocationWire::Url("http://example.com/stream.mp3".into()),
        prefetch_frames: 8192,
    };
    let bytes = encode(&msg);
    assert_eq!(bytes[0], 0x0f);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_opened_roundtrip() {
    let msg = Message::DecoderOpened {
        spec: PcmSpecWire {
            sample_rate_hz: 44100,
            channels: 2,
        },
        duration_ms: Some(240000),
        seekable: true,
        metadata: TrackMetadata {
            title: Some("Test Track".into()),
            artist: Some("Test Artist".into()),
            ..Default::default()
        },
    };
    let bytes = encode(&msg);
    // Variant index 16 = 0x10
    assert_eq!(bytes[0], 0x10);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_opened_minimal_roundtrip() {
    let msg = Message::DecoderOpened {
        spec: PcmSpecWire {
            sample_rate_hz: 48000,
            channels: 1,
        },
        duration_ms: None,
        seekable: false,
        metadata: TrackMetadata::default(),
    };
    let bytes = encode(&msg);
    assert_eq!(bytes[0], 0x10);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_seek_roundtrip() {
    let msg = Message::DecoderSeek { to_ms: 123456 };
    let bytes = encode(&msg);
    // Variant index 17 = 0x11
    assert_eq!(bytes[0], 0x11);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_seeked_roundtrip() {
    let msg = Message::DecoderSeeked { reached_ms: 123400 };
    let bytes = encode(&msg);
    // Variant index 18 = 0x12
    assert_eq!(bytes[0], 0x12);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_format_roundtrip() {
    let msg = Message::DecoderFormat {
        spec: PcmSpecWire {
            sample_rate_hz: 96000,
            channels: 6,
        },
    };
    let bytes = encode(&msg);
    // Variant index 19 = 0x13
    assert_eq!(bytes[0], 0x13);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_stats_roundtrip() {
    let msg = Message::DecoderStats {
        skipped_packets: 42,
        bitrate_kbps: Some(320),
    };
    let bytes = encode(&msg);
    // Variant index 20 = 0x14
    assert_eq!(bytes[0], 0x14);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_stats_no_bitrate_roundtrip() {
    let msg = Message::DecoderStats {
        skipped_packets: 0,
        bitrate_kbps: None,
    };
    let bytes = encode(&msg);
    assert_eq!(bytes[0], 0x14);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_eos_roundtrip() {
    let msg = Message::DecoderEos;
    let bytes = encode(&msg);
    // Variant index 21 = 0x15 (unit variant)
    assert_eq!(bytes, [0x15]);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn decoder_error_roundtrip() {
    let msg = Message::DecoderError {
        message: "decoder initialization failed".into(),
    };
    let bytes = encode(&msg);
    // Variant index 22 = 0x16
    assert_eq!(bytes[0], 0x16);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_configure_roundtrip() {
    let msg = Message::DspConfigure {
        spec: PcmSpecWire {
            sample_rate_hz: 44100,
            channels: 2,
        },
        stages: vec![
            DspStageDesc {
                id: "gain".into(),
                params: vec![0x00, 0x00, 0x80, 0x3f], // 1.0f32
            },
            DspStageDesc {
                id: "eq".into(),
                params: vec![],
            },
        ],
    };
    let bytes = encode(&msg);
    // Variant index 23 = 0x17
    assert_eq!(bytes[0], 0x17);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_configure_empty_stages_roundtrip() {
    let msg = Message::DspConfigure {
        spec: PcmSpecWire {
            sample_rate_hz: 48000,
            channels: 2,
        },
        stages: vec![],
    };
    let bytes = encode(&msg);
    assert_eq!(bytes[0], 0x17);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_ready_roundtrip() {
    let msg = Message::DspReady {
        latency_frames: 1024,
    };
    let bytes = encode(&msg);
    // Variant index 24 = 0x18
    assert_eq!(bytes[0], 0x18);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_reset_roundtrip() {
    let msg = Message::DspReset;
    let bytes = encode(&msg);
    // Variant index 25 = 0x19 (unit variant)
    assert_eq!(bytes, [0x19]);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_reset_ack_roundtrip() {
    let msg = Message::DspResetAck;
    let bytes = encode(&msg);
    // Variant index 26 = 0x1a (unit variant; RFC 0015)
    assert_eq!(bytes, [0x1a]);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn dsp_error_roundtrip() {
    let msg = Message::DspError {
        message: "stage 'eq' not found".into(),
    };
    let bytes = encode(&msg);
    // Variant index 27 = 0x1b (moved by RFC 0015's DspResetAck at 0x1a)
    assert_eq!(bytes[0], 0x1b);
    assert_eq!(decode(&bytes), msg);
}

#[test]
fn all_hosting_variants_roundtrip() {
    let messages = [
        Message::DecoderOpen {
            location: MediaLocationWire::File("test.mp3".into()),
            prefetch_frames: 1024,
        },
        Message::DecoderOpen {
            location: MediaLocationWire::Url("http://test/stream".into()),
            prefetch_frames: 2048,
        },
        Message::DecoderOpened {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            duration_ms: Some(180000),
            seekable: true,
            metadata: TrackMetadata::default(),
        },
        Message::DecoderSeek { to_ms: 5000 },
        Message::DecoderSeeked { reached_ms: 5000 },
        Message::DecoderFormat {
            spec: PcmSpecWire {
                sample_rate_hz: 48000,
                channels: 2,
            },
        },
        Message::DecoderStats {
            skipped_packets: 10,
            bitrate_kbps: Some(256),
        },
        Message::DecoderEos,
        Message::DecoderError {
            message: "test error".into(),
        },
        Message::DspConfigure {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            stages: vec![DspStageDesc {
                id: "test".into(),
                params: vec![1, 2, 3],
            }],
        },
        Message::DspReady {
            latency_frames: 512,
        },
        Message::DspReset,
        Message::DspResetAck,
        Message::DspError {
            message: "dsp error".into(),
        },
    ];

    // The array lists `DecoderOpen` twice on purpose (a file location and a URL),
    // so it has one more entry than there are hosting variants. What matters is
    // that it covers every one of them.
    let distinct: std::collections::BTreeSet<u8> = messages.iter().map(|m| encode(m)[0]).collect();
    assert_eq!(
        distinct.len(),
        HOSTING_VARIANT_COUNT,
        "every hosting variant (RFC 0006 plus RFC 0015's ack) must appear in this round-trip list"
    );

    for msg in messages {
        let bytes = encode(&msg);
        let decoded = decode(&bytes);
        assert_eq!(msg, decoded, "round-trip failed for {:?}", msg);
    }
}

#[test]
fn hosting_variant_indices_are_sequential_after_rfc_0005() {
    // The first hosting variant (DecoderOpen) should be at index 15 (0x0f)
    // because RFC 0005 ended with CommandToHost at index 14 (0x0e). The
    // block is RFC 0006's variants plus RFC 0015's DspResetAck beside
    // DspReset, so DspError is the last one at index 27 (0x1b).
    let indices: Vec<u8> = [
        Message::DecoderOpen {
            location: MediaLocationWire::File("x".into()),
            prefetch_frames: 0,
        },
        Message::DecoderOpened {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            duration_ms: None,
            seekable: false,
            metadata: TrackMetadata::default(),
        },
        Message::DecoderSeek { to_ms: 0 },
        Message::DecoderSeeked { reached_ms: 0 },
        Message::DecoderFormat {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
        },
        Message::DecoderStats {
            skipped_packets: 0,
            bitrate_kbps: None,
        },
        Message::DecoderEos,
        Message::DecoderError {
            message: "x".into(),
        },
        Message::DspConfigure {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            stages: vec![],
        },
        Message::DspReady { latency_frames: 0 },
        Message::DspReset,
        Message::DspResetAck,
        Message::DspError {
            message: "x".into(),
        },
    ]
    .iter()
    .map(|msg| encode(msg)[0])
    .collect();

    // Should be 0x0f through 0x1b (15 through 27)
    let expected: Vec<u8> = (15..=27).collect();
    assert_eq!(
        indices, expected,
        "hosting variants must be appended sequentially"
    );
}

#[test]
fn total_message_variant_count() {
    // Pre-0006: 15 variants (0..14)
    // Hosting block: 13 variants (15..27) - RFC 0006's twelve plus RFC
    // 0015's DspResetAck beside DspReset, which moved DspError to 27
    // The track plug-ins read: NowPlaying at 28
    // RFC 0024: EmbedSkin at 29, OutputClock at 30
    // Total: 31 variants (0..30)
    let all_indices: std::collections::BTreeSet<u8> = [
        Message::Hello {
            protocol_version: 7,
            pid: 0,
        },
        Message::Ping(0),
        Message::Pong(0),
        Message::LoadPlugin {
            path: "x".into(),
            kind: rustamp_plugin_abi::PluginKind::Input,
        },
        Message::PluginLoaded {
            plugin_id: 0,
            name: "x".into(),
            kind: rustamp_plugin_abi::PluginKind::Input,
        },
        Message::PluginError {
            path: "x".into(),
            message: "x".into(),
        },
        Message::PlayerStateUpdate(Box::default()),
        Message::CommandFromPlugin(rustamp_proto::command::Command::Play),
        Message::Vis(Box::new(rustamp_ipc::VisFrameWire {
            timestamp_us: 0,
            waveform: vec![0.0; rustamp_proto::vis::VIS_CHANNELS * rustamp_proto::vis::VIS_SAMPLES],
            spectrum: vec![0.0; rustamp_proto::vis::VIS_CHANNELS * rustamp_proto::vis::VIS_SAMPLES],
        })),
        Message::RingOffer {
            name: "x".into(),
            kind: rustamp_ipc::shm::RingKind::Pcm,
            frame_size: 8,
            capacity: 64,
            size: 512,
            generation: 1,
            direction: rustamp_ipc::shm::RingDirection::MainToHost,
        },
        Message::RingReady {
            kind: rustamp_ipc::shm::RingKind::Pcm,
            generation: 1,
        },
        Message::RingRefused {
            kind: rustamp_ipc::shm::RingKind::Pcm,
            generation: 1,
            reason: rustamp_ipc::shm::RingRefuseReason::MappingUnavailable,
        },
        Message::Shutdown,
        Message::WinampIpc {
            msg_id: 0,
            wparam: 0,
            lparam: 0,
        },
        Message::CommandToHost(rustamp_proto::command::Command::Play),
        // RFC 0006 variants
        Message::DecoderOpen {
            location: MediaLocationWire::File("x".into()),
            prefetch_frames: 0,
        },
        Message::DecoderOpened {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            duration_ms: None,
            seekable: false,
            metadata: TrackMetadata::default(),
        },
        Message::DecoderSeek { to_ms: 0 },
        Message::DecoderSeeked { reached_ms: 0 },
        Message::DecoderFormat {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
        },
        Message::DecoderStats {
            skipped_packets: 0,
            bitrate_kbps: None,
        },
        Message::DecoderEos,
        Message::DecoderError {
            message: "x".into(),
        },
        Message::DspConfigure {
            spec: PcmSpecWire {
                sample_rate_hz: 44100,
                channels: 2,
            },
            stages: vec![],
        },
        Message::DspReady { latency_frames: 0 },
        Message::DspReset,
        Message::DspResetAck,
        Message::DspError {
            message: "x".into(),
        },
        Message::NowPlaying {
            position: 0,
            title: "x".into(),
            file: "x".into(),
        },
        Message::EmbedSkin {
            width: 0,
            height: 0,
            rgba: vec![],
        },
        Message::OutputClock { played_ms: 0 },
    ]
    .iter()
    .map(|msg| encode(msg)[0])
    .collect();

    assert_eq!(all_indices.len(), 31, "exactly 31 variants (0..30)");
    assert_eq!(all_indices.iter().next(), Some(&0x00));
    assert_eq!(all_indices.iter().next_back(), Some(&0x1e));
}

#[test]
fn the_output_clock_round_trips() {
    let message = Message::OutputClock { played_ms: 90_061 };
    let bytes = rustamp_ipc::encode(&message).unwrap();
    assert_eq!(rustamp_ipc::decode(&bytes).unwrap(), message);
}

#[test]
fn the_embed_skin_round_trips() {
    let message = Message::EmbedSkin {
        width: 2,
        height: 1,
        rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
    };
    let bytes = rustamp_ipc::encode(&message).unwrap();
    assert_eq!(rustamp_ipc::decode(&bytes).unwrap(), message);
}
