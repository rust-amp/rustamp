# M15: Engine plays through the decoder resident

## Boundary and scope

Write only `crates/rustamp-audio/**` and this card in the assigned worktree.
Nothing under `D:\code\Rustamp` was read or written; no dirty-room/reference
material. Sources: RFCs 0003, 0006 and 0007, the M10/M12/M14 cards and the
existing clean repository code only. No copied third-party code, no
downloads, no commits, no windows, no audio playback, no host test execution.

One deliberate exception to "owned paths only": registering the two
workspace-internal dependency edges required by the adapter added two lines to
`crates/rustamp-audio/Cargo.toml` (inside the owned path) and its automatic
two-line edge delta in `Cargo.lock`. Both edges point at crates already in the
workspace (`rustamp-mediahost`, `rustamp-proto`); no new external version, no
new package, no download — the same class of build plumbing M12 reported for
its workspace registration. See "Files and line counts".

## Seam chosen: an adapter, not a second decode source

The engine plays through the resident by presenting the resident session as
the core `Decoder` the engine already drives — exactly the seam RFC 0006 §5
names ("wraps the whole conversation in a struct implementing `Decoder`, so
the engine does not learn a second code path").

- `HostDecoder<F: PcmRings>` (`crates/rustamp-audio/src/resident.rs`) wraps a
  `DecoderSession<F>` and implements `rustamp_core::audio::Decoder`.
- `ProcessDecoder` (Windows) owns a spawned `DecoderProcess` and delegates to
  its session with the identical translations.
- `ResidentFactory` implements `DecoderFactory`: registering it in the
  registry is the opt-in. `ResidentConfig` names the helper binary explicitly.

Why this seam: it is the smallest honest one. `decode.rs`, `engine.rs`,
`ring.rs` and `convert.rs` are **untouched** — the gapless loop, the seek
drain, the format-change flush, the vis tap and the audio-clock bookkeeping
cannot diverge between sources because there is only one code path. A second
decode source in the decode thread would duplicate every one of those
invariants. The in-process path keeps working and stays selectable: the
default registry (`rustamp-decoders::default_registry`) contains no resident
factory, so a default engine decodes exactly as before. The resident is
opt-in only (an explicit `ResidentConfig` plus an explicit registration),
never a silent default: selecting it requires naming a helper binary, and a
missing binary degrades loudly (see below) instead of falling back silently.

## Requirement audit

| Requirement | Evidence (final worktree files) |
|---|---|
| Format handshake | `DecoderSession::open` completes `DecoderOpen` → `DecoderOpened` + `RingOffer`/`RingReady` before the adapter exists; `HostDecoder::new` replays `spec`/`duration_ms`/`seekable`/`metadata` from it. Pinned by `handshake_replays_and_transfer_is_bit_exact_with_sticky_eos` (`src/resident.rs`, hosted tests) and `resident_track_plays_bit_exact_to_sticky_eos` (`tests/resident.rs`, engine level: `source_spec`, `duration_frames`). |
| Seek | `convert_seek` sends `DecoderSeek(to_ms)` with `seek_timeout`, attaches only the offer preceding `DecoderSeeked`, refreshes the cached spec. Pinned by `seek_lands_and_reads_the_target` (adapter) and `seek_in_resident_track_plays_the_target` (engine, bit-exact post-seek audio). |
| Sticky EOS only after the ring is drained | Forwarded from the session, which exposes `Ok(0)` only after the drained closed ring *and* `DecoderEos`, and stays at zero. Pinned by the sticky-EOS legs of both bit-exact tests (a second read stays `0`) and by the engine finish condition (run ends only when `consumed >= written`). |
| Loud refusal | `DecoderSession::open` failure surfaces as `CoreError::Decode` (adapter) / `UnsupportedFormat` (missing helper binary); never `Ok(0)`. Pinned by `refusal_is_loud_and_never_eos` (both levels) and `resident_refusal_is_a_loud_open_error` (factory level, no spawn). |
| Loud death | Session `Disconnected` (control EOF, missed `Pong`) maps to `CoreError::Decode`. Pinned by `death_is_loud_and_never_silent_eos` (mid-stream failure after good frames, host ends `Err`) and `control_loss_maps_to_loud_decode_error` (mapping). |
| Bounded wait | Every blocking call uses `Some(timeout)`: `read_timeout` per `read`, `seek_timeout` per seek handshake. Pinned by `bounded_wait_never_hangs_and_retry_delivers_pcm` (20 ms bound fails loudly, retry delivers PCM). Control handshakes inherit the framework's 5 s `CONTROL_WAIT`. |
| Ring drain and clock on source change mid-run | No new logic: both track-change (next queue entry opens a new decoder via the registry, `prepare_track`/converter continue-or-flush as for any two tracks) and intra-track `FormatChanged` (`on_format_changed` flushes DSP tail + converter, engine gets the event, clock untouched) run through the shared decode-thread code. Pinned by `resident_to_inprocess_source_change_is_gapless` (mixed resident→in-process queue: exact seam, continuous timeline, `PlaylistEnded` at the joint total) and `format_change_maps_to_core_format_changed_after_draining_old_pcm` (old PCM drains before the new channel count, one exactly-once `FormatChanged { from, to }`). |
| Wire unchanged | No `Message`, ring-layout, generation or `PROTOCOL_VERSION` change: `ipc`/`proto` untouched, version stays 3. |

## How a failure ends

All paths are loud and end the track, never the player (the decode thread
reports `DecodeOut::Error` on the bus and advances or ends the run exactly as
for a local decode error):

- Undecodable/empty file: `open` fails before any ring exists → `Decode`
  (adapter/factory) or `UnsupportedFormat` (helper binary itself missing, per
  the RFC "format unsupported" degradation).
- Mid-stream damage or truncated stream error: session `OperationFailed` →
  `Decode`; the frames already delivered stay delivered.
- Resident death (process kill, control EOF, missed `Pong`): session
  `Disconnected` → `Decode`; the blocked read wakes via the framework's
  close-only abort, never as EOS.
- Wedged resident: per-`read`/seek timeout → `Decode`; retry works once the
  resident answers again.
- Non-UTF-8 local path with the resident selected: loud `Decode` (the
  framework's strict conversion; in-process decoding is not silently
  substituted). `Reader` sources are refused with `Unsupported` before any
  spawn (RFC 0006 §5.1); the engine queue holds only paths and URLs, so this
  never triggers there.
- Player drop: `HostDecoder`/`ProcessDecoder` drop closes the ring and sends
  `Shutdown`; `DecoderProcess` drop kills and reaps the child. One helper
  process per open decoder, as RFC §5 requires.

## Tests (written, not executed here)

Headless, fake resident, no process spawn in any unit test (byte-stream
control pairs exercise the production framing and the real named rings in one
process, like the framework suite). All adapter tests live in
`src/resident.rs`; engine tests in `tests/resident.rs` (both `#![cfg(windows)]`
where named rings are used; pure mapping tests are cross-platform):

| Test | What it proves |
|---|---|
| `wire_spec_converts_to_core_spec`, `durations_quantize_to_whole_milliseconds`, `timeouts_and_disconnects_are_loud_decode_errors`, `refusals_are_loud_and_never_silent`, `factory_claims_only_resident_extensions`, `reader_sources_are_refused_without_spawning`, `zero_prefetch_is_rejected_loudly` | Pure mapping/config: spec conversion, ms quantization, loud error mapping, EXTENSION-only probe, no spawn for readers/zero capacity |
| `handshake_replays_and_transfer_is_bit_exact_with_sticky_eos` | Handshake replay + full-file bit-identity (`to_bits`), `frames_dropped() == 0`, sticky EOS |
| `seek_lands_and_reads_the_target` | `seek(29 ms)` reaches 29 and the next frame is exact |
| `format_change_maps_to_core_format_changed_after_draining_old_pcm` | Old PCM drains before the new channel count; exactly one core `FormatChanged { 1000 Hz/2 ch -> 2000 Hz/1 ch }` |
| `refusal_is_loud_and_never_eos` | Garbage open is `OperationFailed`, host ends `Err`, never EOS |
| `death_is_loud_and_never_silent_eos` | Mid-stream failure is `Decode` after good frames, host ends `Err` |
| `bounded_wait_never_hangs_and_retry_delivers_pcm` | 20 ms bound fails loudly without hanging; retry delivers PCM |
| `control_loss_maps_to_loud_decode_error` | `Disconnected` maps to `Decode`, never silence/EOS |
| `resident_track_plays_bit_exact_to_sticky_eos` (engine) | Engine replays handshake, renders 3000 frames bit-exact, finishes only on ring drain, freezes at content end, `PlaylistEnded` |
| `seek_in_resident_track_plays_the_target` (engine) | Engine seek settles; post-seek audio is the pattern at the position |
| `resident_to_inprocess_source_change_is_gapless` (engine) | Mixed resident→in-process queue: exact seam, no silence, continuous timeline, joint finish |
| `resident_refusal_is_a_loud_open_error` (engine file, factory level) | Refusal is `Decode` without spawning |
| `seek_past_the_end_clamps_through_the_wire` (engine) | Clamped seek lands within 1 ms of the duration (wire quantization) and terminates with an empty ring |
| `resident_commands_reach_the_engine_unchanged` (engine) | Pause/resume/seek transport behaviour is identical on resident tracks |

### What needs the VM

- Everything above was compiled by Clippy but **not executed on this host**
  (lane constraint). The Windows-gated tests run in the Hyper-V test VM like
  the rest of the suite; none is `#[ignore]`d (one-process, like the
  framework's own transfer tests).
- Additionally requiring **two real processes** (therefore the VM, `#[ignore]`d
  pattern like `pcm_process`): spawning the real
  `rustamp-dechost-symphonia` binary through `ResidentFactory`/`ProcessDecoder`
  (bit-identity against in-process decode of `RUSTAMP_TESTDATA` media), an
  abrupt resident-process kill mid-stream (next read must be loud `Decode`,
  never EOS), and the missing-helper degradation (`UnsupportedFormat`).
- The `decoder_fixture` example + `pcm_process` template in `rustamp-mediahost`
  is the pattern for those two-process cases; no new two-process test was added
  here because the resident binary speaks the identical wire the adapter
  already drives in-process.

## Deliberately left (not repeated from M14)

- No DSP-host adapter: `dsp.rs`/`dsphost` untouched. M14 F1 (infallible DSP
  boundary needs a core-contract RFC) is out of scope and unchanged by this
  lane; nothing here makes it better or worse.
- Truncated-file short-EOS ambiguity (M14 F2) is inherited, not re-litigated:
  the adapter forwards whatever the resident reports (short EOS *or* loud
  error); the requirement it pins is that the host ends instead of hanging.
- `DspReset` wake (M14 F3) and the serve-level generation-reuse guard
  (M14 D4): untouched paths, no new claims.
- No test double drift (M14 F4): the adapter tests use the production
  `NamedPcmRings`, not a weaker double.
- URL refusal (M14 F5/F-C8 lane): the factory forwards URLs to the resident,
  which refuses them loudly per M12 (M4 territory); no new URL test is claimed.
- Engine retry/restart policy stays exactly what it is for local decoders (a
  failed track reports and the run moves on); no new policy was added.
- Packaging (`xtask package` shipping the helper, missing-helper degradation
  in installs) belongs to a later card.

## Own choices (everything not dictated by a spec or RFC)

1. Adapter over second decode source: RFC 0006 §5 names it; zero changes to
   `decode.rs`/`engine.rs` were needed or made.
2. Opt-in via registry registration with `Confidence::EXTENSION` on the
   symphonia-format list; ties break by registration order (RFC 0013): first
   wins for resident-first, last for in-process-first, alone to force.
   The default registry is untouched, so the default engine is unchanged.
3. `DEFAULT_PREFETCH_FRAMES = 8192`, `DEFAULT_READ_TIMEOUT` /
   `DEFAULT_SEEK_TIMEOUT = 5 s`: long enough for a loaded helper, short
   enough that a wedge fails the track instead of hanging it.
4. Explicit helper path, no `PATH` lookup; missing binary →
   `UnsupportedFormat` (RFC "format unsupported"), refusal/death/wedge →
   `Decode` (loud, track fails, player survives).
5. Error map: every media-host failure becomes `Decode` except the translated
   `FormatChanged { from, to }`; misaligned/empty read buffers are
   `InvalidArgument` (caller bug, checked before touching the session).
6. Millisecond quantization is wire truth: seeks round to whole ms and the
   past-the-end clamp lands within 1 ms of the duration (pinned by test).
7. `Reader` refused with `Unsupported` before any spawn; non-UTF-8 refused
   loudly with no silent in-process fallback.
8. `ProcessDecoder` holds its child behind a mutex: the core trait splits
   `&self`/`&mut self` while the session needs exclusivity. The decode worker
   is the sole accessor (never contended, never the audio thread); poison maps
   to `Decode`.
9. The engine-level `FakeResidentFactory`/`ReapedDecoder` live in the test
   file only and are never shipped; production uses `ResidentFactory` with a
   real helper binary.
10. The `Cargo.lock` two-line edge delta (no new package, no new version) is
    automatic build plumbing for the two workspace-internal edges, in the
    same class as M12's registration delta.

## Files and line counts

Blank lines included. `Cargo.toml` gain is inside the owned path; the
`Cargo.lock` delta is reported above rather than slipped in.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-audio/src/resident.rs` | 996 | new: `ResidentConfig`, `HostDecoder`, `ProcessDecoder` (Windows), `ResidentFactory`, mapping helpers, 7 pure + 8 hosted tests |
| `crates/rustamp-audio/tests/resident.rs` | 570 | new: fake resident + 6 engine tests (Windows, headless, no spawn) |
| `crates/rustamp-audio/src/lib.rs` | 63 | +4: `resident` module + re-exports |
| `crates/rustamp-audio/Cargo.toml` | 22 | +2: workspace-internal `rustamp-mediahost`, `rustamp-proto` edges (no new external crate) |
| `Cargo.lock` | +2 | automatic edge delta for the above; no new package or version |
| this card | new | owned doc path |

No file under `rustamp-core`, `rustamp-ipc`, `rustamp-proto`, `rustamp-decoders`,
`rustamp-dechost`, `rustamp-mediahost`, `rustamp-app`, `rustamp-ui` or `xtask`
was touched; the wire and `PROTOCOL_VERSION` are unchanged.

## Compile-only gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, includes the new unit, hosted and engine tests.
- `cargo build --offline --workspace`: **exit 0** (~54 s).
- One Clippy MSRV finding fixed on the way (`manual_is_multiple_of`:
  `is_multiple_of`, per the CONTRIBUTING MSRV note); no foreign-lane files
  were modified to make a gate pass.
- **No `cargo test`** (not even `--no-run` binaries beyond what Clippy
  compiles), no test process spawned, no example run, no window opened, no
  audio played, no download, no dependency addition, no install, no commit,
  no git write. Only `cargo fmt`, `cargo clippy`, `cargo build` and
  read-only inspection commands were used, all with `--offline` where the
  tool supports it.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read.
  No third-party code copied. Worktree-local `git status --short` is the
  final boundary receipt below; it lists exactly the files above (plus the
  automatic lock edge) and nothing under `D:\code\Rustamp` (which was never
  accessed).
