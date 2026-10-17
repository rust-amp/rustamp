# M12: First real decoder resident (symphonia behind the media host)

## Boundary and scope

Write only `crates/rustamp-mediahost/**`, `crates/rustamp-dechost/**`,
`crates/rustamp-dsphost/**` and this card in the assigned worktree. Nothing
under `D:\code\Rustamp` was read or written; no dirty-room/reference
material. Sources: RFCs 0003, 0006, 0007, the M10 card and the existing clean
repository code only. No copied third-party code, no dependency additions, no
downloads, no commits, no windows, no audio playback, no host test execution.

One deliberate exception to "owned paths only": registering the new crate
required adding one line to the workspace `Cargo.toml` members list
(`crates/rustamp-dechost`) and its path-only entry in `Cargo.lock`. Both
deltas are pure build plumbing (a local package, no new external version, no
download); see "Files and line counts". No other unowned file was touched.

## 1. What the decoder host did before, and does now

There are no `rustamp-dechost` / `rustamp-dsphost` crates on `main`: both
helpers live as binaries under `rustamp-mediahost` (`src/bin/dechost.rs`,
`src/bin/dsphost.rs`).

### Before (baseline, all in `crates/rustamp-mediahost`)

Stub:

- `src/lib.rs:69-121` (`DecoderHost`): `Hello`/`Ping` answered, `Shutdown`
  accepted; `DecoderOpen` (`:101`) and `DecoderSeek` (`:107`) return
  `NotImplemented`. Real only as a protocol-shape placeholder.
- `src/lib.rs:124-167` (`DspHost`): same for `DspConfigure` (`:150`) /
  `DspReset`. Unchanged by this lane.
- `src/bin/dechost.rs:1-23`: serves `UnavailableResident`
  (`src/decoder.rs:58-67`), i.e. every open fails with "no decoder resident
  linked". The framework explicitly reports no resident instead of decoding.
- `src/bin/dsphost.rs:1-45`: control-protocol skeleton; configure answers
  `DspError`. Unchanged by this lane.

Real (M10 framework, reused unchanged):

- `src/control.rs`: `ControlPeer::start` (`:76`) spawns framed reader/writer
  threads plus a `Hello`/`Ping`/`Pong` monitor (`:126`); EOF, missed `Pong`,
  wrong-version `Hello` or a stalled consumer all fail sticky
  (`:142-208`) and close the registered ring view, waking blocked PCM.
- `src/decoder.rs:160-174` (`serve_decoder`): open handshake, ring offer,
  `DecoderOpened`, pump, seek/format/EOS/error transitions, `Shutdown`.
  `pump` (`:102-156`) fills with infinite lossless `write_from_blocking`
  (`:146`) and checks control between blocks; `serve_inner` (`:176-274`)
  closes the old ring before any replacement (`:237`) and sends `DecoderEos`
  only after the final block is published (`:238-242`).
- `src/decoder.rs:289-570` (`DecoderSession`): player-side handshake, `read`
  (`:442`: EOS only after a drained closed ring *and* `DecoderEos`; an
  explicit timeout is `TimedOut`, never `Ok(0)`), `seek` (`:512`: drops
  prefetch, attaches only the offer preceding `DecoderSeeked`),
  `Drop` sends `Shutdown` (`:565`).
- `src/pcm.rs:58-167` (`NamedPcmRings`): host creates/offers the decoder
  ring with wakes before `RingReady` (`:67`); player validates kind,
  direction, size, format and generation before opening (`:116`).

### Now (this lane)

New crate `crates/rustamp-dechost` (GPL-3.0-or-later, no §7 exception: it
links the GPL decoder crate and Symphonia, so it cannot be permissive):

- `src/lib.rs:37-45` (`RegistryResident`): implements the framework's
  `DecoderResident`. `open` (`:107-121`) opens a real file through
  `rustamp-decoders::default_registry()` and wraps it in `RegistryDecoder`;
  `Url` locations are refused with a clean terminal error (network streaming
  is M4; `Reader` sources cannot cross the boundary, RFC 0006 §5.1).
- `src/lib.rs:47-65` (`RegistryDecoder`): adapts a live core `Decoder`.
  `spec_wire` (`:68-84`) converts `AudioSpec` to `PcmSpecWire` and rejects
  zero/oversized values loudly so no unvalidatable ring is ever offered.
  `read` (`:132-137`) forwards frames; a core `FormatChanged` stores the
  already-switched spec and becomes the framework's unit signal, anything
  else becomes `OperationFailed` via `map_read_error` (`:86-100`). `seek`
  (`:139-146`) forwards, refreshes the spec and reports reached
  milliseconds (the inner decoder clamps past-the-end targets to the
  duration). `info` (`:123-130`) replays spec/duration/seekability/metadata;
  `stats` (`:148-153`) replays skipped packets and bitrate, so the
  framework's change-only `DecoderStats` path is fed with real telemetry.
- `src/main.rs:8-22`: binary `rustamp-dechost-symphonia` — `ControlPeer` over
  stdio plus `serve_decoder` with `RegistryResident` and `NamedPcmRings`.
  The name differs from the framework placeholder `rustamp-dechost` on
  purpose (two same-named binaries would collide in `target/debug/`); the
  placeholder stays for the framework tests, packaging picks this binary.
- `src/lib.rs:155-199`: pure unit tests for the spec conversion and the
  error mapping. No I/O, no window, no audio.

Nothing in `rustamp-mediahost`, the wire, or any other crate was modified.

## 2. Tests (written, not executed here)

`crates/rustamp-dechost/tests/real_decode.rs` (`#![cfg(windows)]`, one
headless process, real named rings in-process, small capacities 3/7 so every
run wraps). Byte-stream control pairs exercise production framing.

| Test | What it proves |
|---|---|
| `real_file_opens_with_format_handshake_and_decodes_bit_exact` | Handshake (`44_100 Hz`/stereo, `duration_ms 1500`, seekable, real title metadata) plus full-file transfer bit-identical (`to_bits`) to the same decoder in-process, `frames_dropped() == 0`, sticky EOS |
| `end_of_stream_comes_only_after_the_ring_is_drained` | Every expected frame arrives before the first `Ok(0)`; a second read stays `0`. Capacity 3 |
| `seek_inside_matches_direct_decode` | `seek(1000 ms)` reaches 1000 and the next block equals direct decode at the same offset |
| `seek_past_the_end_clamps_and_terminates` | `seek(60_000 ms)` clamps to the duration and the next read is `Ok(0)`, no hang |
| `garbage_and_empty_files_are_clean_errors_never_eos` | 256 junk bytes and a 0-byte file both fail `open` with `OperationFailed`; the host thread ends with `Err`, never EOS |
| `truncated_file_ends_without_hanging` | A valid WAV header cut mid-data terminates (short EOS *or* loud error — either is a clean end) with fewer than the full 4000 frames; the host joins, never spins |
| `dropping_the_player_stops_the_host_cleanly` | Session drop (ring close + `Shutdown`) ends the host with `Ok` |

Waiting rule (no repeat of the M10 flakiness): PCM reads use `None` —
condition waits woken by data, EOS or terminal failure, never by a clock.
Only joining the host thread uses a generous 30 s bound (`JOIN_WAIT`), which
a healthy host reaches in milliseconds; the expiry message names the stall.
`open`/`seek` inherit the framework's 5 s `CONTROL_WAIT` for a single
handshake exchange (unchanged framework constant).

### What needs the VM

Everything above is written for the Hyper-V test VM and was **not executed
on this host** (lane constraint). Additionally requiring two real processes
(and therefore the VM): spawning `rustamp-dechost-symphonia` through
`DecoderProcess`, an abrupt resident-process kill mid-stream, and any
`RUSTAMP_TESTDATA` media. The existing `pcm_process` two-process pattern in
`rustamp-mediahost` is the template; no new two-process test was added here
because the resident binary speaks the identical wire the framework tests
already drive across processes.

## 3. Wire change: none

`PROTOCOL_VERSION` stays 3. No message added, removed or reshaped; no field,
refusal reason or ring layout touched. The resident speaks exactly the
existing `DecoderOpen` → `DecoderOpened` + `RingOffer`/`RingReady` →
`DecoderStats?` → `DecoderEos` | `DecoderFormat` → new offer →
`DecoderError` | `Shutdown` conversation. The one structural addition (a new
workspace member depending on four existing workspace crates) is build
plumbing, not protocol, and is reported above rather than slipped in.

## 4. How a failure ends the host

All paths end the helper process without hanging or spinning (the run loop
re-checks control between blocks and after every closed-ring wake):

- Decoder/read error (including corrupt packets past the 512-error budget):
  `pump` returns `Err` → `serve_decoder` publishes `DecoderError`, flushes
  boundedly (`CONTROL_WAIT`), returns `Err`; the binary exits non-zero.
- Undecodable or empty file: `open` fails → same `DecoderError` path before
  any ring exists; the player sees `OperationFailed`, never EOS.
- Truncated file: symphonia ends the stream (`Ok(0)`) or errors; the host
  publishes EOS-then-close or `DecoderError` respectively and stays alive
  for `Shutdown`.
- Seek past the end: clamped to the duration by the inner decoder, then a
  normal EOS; seek of an unseekable stream is refused loudly.
- Player vanishes: control EOF/missed `Pong` fails the peer sticky; the
  close-only abort wakes the blocked writer, `pump` observes the failure and
  the host exits. Graceful drop is `Shutdown` → `Ok(0)` path.

## 5. What is left

- The DSP host: `DspHost`/`dsphost` remain the M10 stub; no DSP resident
  crate was created. Two-ring round trip, latency, reset and reconfigure
  still belong to a later card.
- Codec-resident crates for other formats: this crate serves everything the
  shared registry knows (MP3/FLAC/Vorbis/WAV/AIFF/CAF/AAC/ALAC/MKA today);
  FFI families (Opus, game-music-emu, WavPack, APE, MPC) and URL/network
  sources (M4) need their own residents/sources.
- Engine side: no `rustamp-core::Decoder` adapter over `DecoderSession` and
  no engine registration/retry policy (other lanes own `rustamp-audio`;
  `DecoderSession` at `mediahost/src/decoder.rs:289` is the ready-made
  starting point).
- RFC 0006 deferrals, unchanged: non-UTF-8 path wire form (HANDOFF-3;
  refused loudly here via `MediaLocationWire::File`), URL auth, late join,
  multiplexing, M6 non-Windows wakes/mappings, engine buffering/resampling/
  latency policy, and literal host-side wait-for-drain before EOS (the
  observable order — player `Ok(0)` only after drain — already holds via
  close-then-`DecoderEos`; see M10 §"Literal host-side drain-before-EOS
  ordering").
- Packaging: `xtask package` must ship `rustamp-dechost-symphonia` next to
  the player; a missing helper must degrade to "format unsupported".
- A decoder-registry note for the owning lanes (no action taken here, files
  untouched): nothing in `rustamp-decoders`/`rustamp-core` blocked this
  work; the adapter needed no new registry API.

## 6. Files and line counts

Blank lines included. `Cargo.toml`/`Cargo.lock` deltas are build plumbing.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-dechost/Cargo.toml` | 17 | new |
| `crates/rustamp-dechost/src/lib.rs` | 186 | new |
| `crates/rustamp-dechost/src/main.rs` | 30 | new |
| `crates/rustamp-dechost/tests/real_decode.rs` | 336 | new |
| `Cargo.toml` (workspace members) | +1 | register `crates/rustamp-dechost` |
| `Cargo.lock` | +10 | local `rustamp-dechost` package entry only; no new external version |
| this card | new | owned doc path |

No file under `crates/rustamp-decoders`, `rustamp-core`, `rustamp-ipc`,
`rustamp-audio`, `rustamp-app`, `rustamp-ui` or `rustamp-meta` was touched.

## 7. Compile-only gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0** (compiles all new tests without running them).
- `cargo build --offline --workspace`: **exit 0** (~57 s); both
  `target/debug/rustamp-dechost.exe` (framework placeholder, 0.9 MB) and
  `target/debug/rustamp-dechost-symphonia.exe` (real resident, 5.6 MB) link.
- **No `cargo test`** (not even `--no-run` binaries beyond what clippy
  compiles), no test process spawned, no example run, no window opened, no
  audio played, no download, no dependency addition, no install, no commit,
  no git write. Only `cargo fmt`, `cargo clippy`, `cargo build` and
  read-only inspection commands were used.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read.
  No third-party code copied. Worktree-local `git status --short` is the
  final boundary receipt below; it lists exactly the files above and nothing
  under `D:\code\Rustamp` (which was never accessed).
