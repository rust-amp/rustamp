# RFC 0001: Core contracts (M0)

Status: **Accepted / frozen** at the end of M0.
Owner: Integration Lead. Any change to the public API of `rustamp-core`,
`rustamp-ipc` (the `Message` enum) or `rustamp-plugin-abi` requires a new RFC
in `docs/rfc/` (additive changes included, except adding mocks/helpers
that do not touch existing signatures).

## Goals

Let all lanes (audio, decoders, skin/gfx, UI, playlist, platform, plugin host,
vis) code in parallel against stable types, with mocks available from day one.

## Decisions

- **Dependency direction.** `rustamp-core` depends on no internal crate and
  only on `serde`, `thiserror`, `crossbeam-channel`. `rustamp-ipc` depends on
  core and `rustamp-plugin-abi`. Both build for x86_64 and i686.
- **Units on the wire.** Positions/durations are `u64` ms (`i64` for relative
  seeks). Audio traits use `std::time::Duration` and frame counts (`u64`).
  Indices crossing crate/process boundaries are `u32`; no `usize` in wire types
  so postcard encoding is identical on x64 and i686. Entries are referenced by
  stable `EntryId(u64)` wherever possible.
- **Volume** is `u8` `0..=255` (Winamp style); **balance** `i8` `-127..=127`.
  **EQ** gains are `f32` dB (nominally +-12). Mapping to the Winamp slider
  scale is a UI concern defined by spec S07.
- **Audio format.** Interleaved `f32`, `AudioSpec { sample_rate, channels }`.
  `Decoder::read` returns frames, `0` = EOF. `seek` returns the position
  reached.
- **Decoder discovery.** `DecoderFactory::probe(ProbeHint{extension, header})`
  returns a `Confidence` (0..=100; `EXTENSION` = 40, `CONTENT` = 90).
  `DecoderRegistry::open` sniffs 4 KiB, picks the best score (earliest
  registration wins ties).
- **Output is pull based.** `OutputDevice`: `open(spec) -> OutputFormat`
  (device may grant another rate/channels; the engine adapts), then
  `start(RenderFn)`. `RenderFn` is a real-time callback that fills the whole
  buffer (silence on underrun) and must not allocate/lock; the engine backs it
  with a lock-free SPSC ring. `played_frames()` is the audio clock.
- **DSP.** `DspStage::{prepare, process (in place), reset, latency_frames}`.
- **Vis.** `VisFrame` is 2 x 576 waveform + 2 x 576 spectrum plus a
  timestamp. Contents are neutral (`f32` waveform, linear magnitude). The Winamp
  derivation (8-bit scaling, FFT, bins) is spec **S08** and lives in
  `rustamp-vis`. IPC carries it as `VisFrameWire` (flat vectors).
- **Commands/Events** are plain serde enums. Commands are requests; events are
  facts. Indices are `u32`. `Command::Select(SelectOp)` etc. are listed in
  `command.rs`.
- **Playlist** is a concrete `Playlist` struct (not a trait) with stable ids,
  selection and current entry. Shuffle order, randomize, file formats are
  `rustamp-playlist` behaviors; the core offers `apply_order`.
- **Bus.** Commands: cloneable `CommandSender` -> single `CommandReceiver`.
  Events: `EventBus` broadcast with per-subscriber unbounded channels. Never
  publish from the audio thread.
- **Config.** `Config` root + `AudioConfig`, `UiConfig`, `EqConfig`,
  `PlaylistConfig`, `PluginsConfig`; every struct is `#[serde(default)]` so
  adding fields is backwards compatible. File I/O is a later task.
- **IPC.** `Message` (postcard) with `Hello`, `Ping/Pong`, `LoadPlugin`,
  `PluginLoaded/PluginError`, `PlayerStateUpdate`, `CommandFromPlugin`, `Vis`,
  `Shutdown`, and the placeholder `WinampIpc { msg_id, wparam, lparam }` whose
  semantics come from spec S10. `Transport` trait with `MemoryTransport` and
  `StreamTransport` (u32-LE length-prefixed frames, 16 MiB cap). Shared-memory
  rings for PCM/vis come later (M3). `PROTOCOL_VERSION = 0`.
- **Plugin ABI.** `rustamp-plugin-abi` only has `PluginKind`. `#[repr(C)]`
  structs are added from spec S09 by a clean-room implementer; nothing is
  invented from memory.
- **Mocks.** Feature `mock` of core: `MockDecoder`, `MockDecoderFactory`,
  `NullOutput` (manual `pump`), `MockDsp`.

## Known limitations / open points

- `PathBuf` is serialized as UTF-8 text; non-UTF-8 Windows paths (unpaired
  surrogates) will fail to encode over IPC. Revisit if it matters.
- Windows `WindowId::Gen(u32)` numbering scheme is not yet defined.
- Gains/volume ranges may be refined by specs S02/S07 (would need a new RFC).
