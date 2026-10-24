# M13: First real DSP resident (gain/passthrough chain behind the media host)

## Boundary and scope

Write only `crates/rustamp-mediahost/**`, `crates/rustamp-dsphost/**`
and this card in the assigned worktree. Nothing under `D:\code\Rustamp`
was read or written; no dirty-room/reference material. Sources: RFCs 0003,
0006, 0007, the M10 and M12 cards and the existing clean repository code
only. No copied third-party code, no downloads, no commits, no windows, no
audio playback, no host test execution.

Two deliberate exceptions to "owned paths only", both build plumbing for a
local package (no new external version, no download), mirroring M12:

- one line in the workspace `Cargo.toml` members list
  (`crates/rustamp-dsphost`);
- its path-only entry in `Cargo.lock` (+10 lines: the local
  `rustamp-dsphost 0.0.1` package depending on four workspace crates).

The new resident crate also depends on four existing workspace crates
(`rustamp-core`, `rustamp-ipc`, `rustamp-mediahost`, `rustamp-proto`).
These are path-only edges to code already in the lockfile, not new
dependencies in the crates.io sense; `THIRD_PARTY.md` is untouched because
no external version entered the tree. See "Files and line counts".

`crates/rustamp-dechost`, `crates/rustamp-dsp`, `crates/rustamp-core`,
`crates/rustamp-ipc` and every other crate were not touched (the framework
generalisation needed no change outside the owned paths).

## 1. What the DSP host did before, and does now

There is no `rustamp-dsphost` crate on `main`: the helper lives as a stub
binary under `rustamp-mediahost` (`src/bin/dsphost.rs`).

### Before (baseline, all in `crates/rustamp-mediahost`)

Stub:

- `src/lib.rs:124-167` (`DspHost`): `Hello`/`Ping` answered, `Shutdown`
  accepted; `DspConfigure` (`:150`) and `DspReset` (`:154`) return
  `NotImplemented`. Real only as a protocol-shape placeholder.
- `src/bin/dsphost.rs` (old, 45 lines): its own ad-hoc control loop over
  `StreamTransport`; configure answers `DspError`. No rings, no chain.

Real (M10 framework, reused and generalised, not duplicated):

- `src/control.rs`: `ControlPeer::start` spawns framed reader/writer
  threads plus a `Hello`/`Ping`/`Pong` monitor; EOF, missed `Pong`,
  wrong-version `Hello` or a stalled consumer all fail sticky and close
  the registered ring view, waking blocked PCM.
- `src/decoder.rs` (`serve_decoder`, `DecoderSession`): the open
  handshake, ring offer, pump, seek/format/EOS/error transitions and
  `Shutdown` that this lane mirrors for DSP.
- `src/pcm.rs`: host creates/offers the decoder ring with wakes before
  `RingReady`; the player validates kind, direction, size, format and
  generation before opening.

### Now (this lane)

New crate `crates/rustamp-dsphost` (GPL-3.0-or-later, no §7 exception: it
links `rustamp-core` DSP contracts and exists to isolate DSP code, so it
cannot be permissive):

- `src/lib.rs` (`StageResident`): implements the framework's
  `DspResident`. `open` (`spec` + `stages`) validates the wire spec loudly,
  builds one `Gain` (`params` = exactly 4 LE bytes holding a finite
  `f32`, the plain per-sample factor) or `Passthrough` (empty params,
  bit-exact bypass) `DspStage` per descriptor in order, prepares each
  against the core `AudioSpec`, and sums `latency_frames` (0 for both
  stages today). Unknown ids and undecodable params fail here with
  `OperationFailed`, before any ring exists, and become `DspError`.
  An empty stage list is a valid zero-latency chain (documented own
  choice). Pure unit tests for spec conversion, param decoding, unknown
  ids, exact gain math, stage order and prepare validation. No I/O, no
  window, no audio.
- `src/main.rs`: binary `rustamp-dsphost-stages` — `ControlPeer` over
  stdio plus `serve_dsp` with `StageResident` and `NamedPcmRings`. The
  name differs from the framework placeholder `rustamp-dsphost` on
  purpose (two same-named binaries would collide in `target/debug/`);
  the placeholder stays for the framework tests, packaging picks this
  binary.
- Framework, `crates/rustamp-mediahost/src/dsp.rs` (new, 787 lines):
  `ResidentChain` / `DspResident` / `UnavailableDspResident`,
  `serve_dsp` (open handshake, two-ring offer, block pump,
  reconfigure/reset/EOS/error/`Shutdown` transitions), player-side
  `DspSession` (`open`/`write`/`read`/`finish_writes`/`reset`/
  `reconfigure`, `spec`/`latency_frames`/`generations`/
  `frames_dropped`), and `Drop` sending `Shutdown`, plus Windows-only
  `DspProcess` (spawn/kill/reap with `CREATE_NO_WINDOW`, mirroring
  `DecoderProcess`).

Small framework generalisations (named, not silent):

1. `src/pcm.rs`: `create_directed` / `open_directed` with a
   `RingDirection` on the `PcmRings` trait (defaults keep the decoder
   `HostToMain` path byte-for-byte), implemented by `NamedPcmRings`;
   the in-ring flows `MainToHost` while the decoder entry points still
   offer `HostToMain` (pinned by `tests/dsp_named.rs`). New `ClosePair`
   abort so one supervisor registration wakes both rings.
2. `src/control.rs`: the monitor now also fails sticky on `DspError`
   (`"resident DSP failed: …"`), and the `interrupt_seek` flag is
   renamed `interrupt_reoffer` and also covers `DspConfigure`, so a
   reconfigure wakes a worker blocked in infinite PCM I/O. `DspReset`
   deliberately never closes rings. Positional callers (both placeholder
   binaries, the decoder fixture, all tests) are unaffected.
3. `src/bin/dsphost.rs`: rewritten onto `ControlPeer` + `serve_dsp`
   with `UnavailableDspResident`, mirroring `dechost.rs`; the `DspHost`
   stub struct and its tests in `src/lib.rs` stay untouched.

Lifecycle (RFC 0006 §6, unchanged wire): `DspConfigure { spec, stages }`
→ player creates/offers the in-ring (`MainToHost`) → host opens it
(`RingReady`) → host creates/offers the back-ring (`HostToMain`) →
player opens it (`RingReady`) → host sends `DspReady { latency_frames }`.
Run: player writes blocks, host processes in order, player reads back.
`DspReset` clears stage state, rings untouched. Reconfigure closes and
re-offers both rings with bumped generations (old unread PCM dies with
the old generation, like a seek discarding prefetch). End of stream: the
player closes its in-ring after the final block (`finish_writes`), the
host publishes everything it read and closes the back-ring, and the
player exposes `Ok(0)` only after that closed back-ring is drained —
then sticks there. A dead host is never silent EOS: the supervisor fails
sticky and every wait turns loud.

Nothing in the wire, `rustamp-core`, `rustamp-dsp` or any other crate was
modified.

## 2. Tests (written, not executed here)

`crates/rustamp-dsphost/tests/dsp_roundtrip.rs` (`#![cfg(windows)]`, one
headless process, real named rings in-process, small capacities 3/7 so
every run wraps). Byte-stream control pairs exercise production framing.

| Test | What it proves |
|---|---|
| `passthrough_round_trip_is_bit_exact_and_eos_is_sticky` | Handshake (`spec` echo, `latency_frames == 0`) plus full 4099-frame sweep bit-identical (`to_bits`) through a wrapping ring, `frames_dropped() == 0`, drain-then-`Ok(0)`, sticky second `Ok(0)` |
| `gain_chain_round…` (`gain_chain_changes_audio_measurably`) | `[passthrough, gain(2.0)]` returns exactly `input * 2.0` per sample, differs from the input (measurable), zero drops, sticky EOS |
| `unknown_stage_is_a_loud_error_before_any_ring` | `DspConfigure` with `fuzz` fails `open` with `OperationFailed("unknown DSP stage")`; the host thread ends with `Err`, never EOS |
| `gain_with_bad_params_is_refused` | 3-byte gain params are refused with `gain params`; host thread ends `Err` |
| `reset_keeps_rings_and_continues` | `DspReset` mid-run leaves both generations unchanged and the stream stays sample-exact across the reset |
| `reconfigure_reoffers_both_rings_with_new_sound` | 500 frames at ×2.0, reconfigure to ×0.5, both generations bump, the rest arrives ×0.5, then sticky EOS |
| `dropping_the_player_stops_the_host_cleanly` | Session drop (`Shutdown`) ends the host with `Ok` |
| `abrupt_player_loss_stops_the_host_without_hanging` | Raw-transport manual handshake, one block, then pipes dropped without `Shutdown`: the host thread ends `Err(Disconnected)`, never `Ok` |
| `abrupt_host_loss_is_loud_and_never_eos` | Fake host dies after one block without closing the ring: the infinite player read wakes with `Disconnected`, never `Ok(0)` |

Waiting rule (no repeat of the M10 flakiness): PCM reads use `None` —
condition waits woken by data, a drained close or a terminal failure,
never by a clock. Only joining the host thread or a woken outcome uses
a generous 30 s bound (`JOIN_WAIT`), which a healthy host reaches in
milliseconds; the expiry message names the stall. `open`/`reconfigure`
inherit the framework's 5 s `CONTROL_WAIT` for single handshake
exchanges. The manual-transport control reads are blocking by API
(`StreamTransport::recv` takes no deadline); each completes on the next
peer message, and the test still cannot hang past the thread-join bound.

Framework-side, in `crates/rustamp-mediahost`:

- `tests/dsp_named.rs` (`#![cfg(windows)]`): directed rings carry both
  directions losslessly with drain-then-`Closed` and zero drops; the
  wrong direction is refused before any mapping opens; the plain
  `create`/`open` entry points still offer `HostToMain` (the decoder
  path did not move); zero/invalid formats are refused.
- `tests/dsp_process.rs` (`#![cfg(windows)]`, both `#[ignore]`, VM
  only): `named_dsp_two_process_gain_round_trip_then_sticky_eos`
  (real `rustamp-dsphost-stages`-shaped helper via `dsp_fixture`:
  ×2.0 round trip, drain, sticky EOS, zero drops) and
  `host_exits_mid_stream_and_infinite_read_survives`
  (`fixture:exit` kills the helper mid-stream; the infinite read fails
  `Disconnected`, never `Ok(0)`).
- `examples/dsp_fixture.rs` + `tests/support/dsp_resident.rs`: the
  deterministic stub resident (`gain`/`passthrough` plus fixture-only
  `fixture:fail` loud refusal and `fixture:exit` crash surrogate),
  mirroring the decoder fixture.

### What needs the VM

Everything above is written for the Hyper-V test VM and was **not
executed on this host** (lane constraint). Additionally requiring two
real processes (and therefore the VM): spawning the real
`rustamp-dsphost-stages` binary through `DspProcess`, an abrupt
resident-process kill mid-stream, and any `RUSTAMP_TESTDATA` media. The
existing `dsp_process` two-process pattern plus the staged
`dsp_fixture` example (`cargo build -p rustamp-mediahost --example
dsp_fixture`, guest `RUSTAMP_DSP_FIXTURE`) is the template, mirroring
`pcm_process`/`RUSTAMP_DECODER_FIXTURE`.

## 3. Wire change: none

`PROTOCOL_VERSION` stays 3. No message added, removed or reshaped; no
field, refusal reason or ring layout touched. The resident speaks
exactly the existing `DspConfigure` → `RingOffer`/`RingReady` ×2 →
`DspReady` → blocks → `DspReset` / reconfigure → `DspError` |
`Shutdown` conversation. The one structural addition (a new workspace
member depending on four existing workspace crates) is build plumbing,
not protocol, and is reported above rather than slipped in.

## 4. How a failure ends the host

All paths end the helper process without hanging or spinning (the run
loop re-checks control between blocks and after every closed-ring
wake; the supervisor's `ClosePair` abort wakes both blocked directions
on control loss, shutdown or reconfigure):

- Unknown stage id / bad params / zero spec: `open` fails → `serve_dsp`
  publishes `DspError`, flushes boundedly (`CONTROL_WAIT`), returns
  `Err`; the binary exits non-zero before any ring exists. The player
  sees `Disconnected("resident DSP failed: …")`, never EOS.
- Player vanishes mid-run (control EOF/missed `Pong`): the peer fails
  sticky; the close-only aborts wake the blocked reader/writer, `pump`
  observes the failure and the host exits with `Err`.
- Player drops the session gracefully: `Shutdown` closes both rings and
  the host exits `Ok(0)`.
- Graceful end-of-writes: the player closes its in-ring; the host
  publishes every read frame, closes the back-ring, and waits for
  `Shutdown` or a fresh configure. The player drains to a sticky `Ok(0)`.
- Reconfigure: both rings close (RAII `Drop` on every early error too),
  a fresh chain opens, fresh generations are checked against reuse, and
  a new `DspReady` reports the new latency.

## 5. What is left

- Engine side: no `rustamp-core::DspStage` adapter over `DspSession` and
  no engine registration/latency-compensation wiring (other lanes own
  `rustamp-audio`; `DspSession` at `mediahost/src/dsp.rs` is the
  ready-made starting point).
- Resident coverage: only `gain` and `passthrough` live in the resident
  today. Equalizer/limiter residents (over `rustamp-dsp`), opaque-param
  stage registries, and classic `dsp_` DLL bridging (`plughost32`,
  S10/S11) belong to later cards.
- Back-ring capacity is a fixed own choice (`BACK_RING_FRAMES = 4096`)
  because `DspConfigure` carries no capacity hint; the in-ring capacity
  is the player's per-configure choice. A wire capacity hint would be a
  contract change (new RFC).
- RFC 0006 deferrals, unchanged: resampling/buffering/latency engine
  policy, late join, multiplexing, M6 non-Windows wakes/mappings, and
  packaging (`xtask package` must ship `rustamp-dsphost-stages` next to
  the player; a missing helper must degrade to "stage unavailable").
- A DSP note for the owning lanes (no action taken here, files
  untouched): nothing in `rustamp-dsp`/`rustamp-core` blocked this work;
  the chain needed no new registry API.

## 6. Files and line counts

Line counts via `splitlines` (code + blanks, no diffstat).
`Cargo.toml`/`Cargo.lock` deltas are build plumbing.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-dsphost/Cargo.toml` | 19 | new |
| `crates/rustamp-dsphost/src/lib.rs` | 340 | new |
| `crates/rustamp-dsphost/src/main.rs` | 32 | new |
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | 466 | new |
| `crates/rustamp-mediahost/src/dsp.rs` | 787 | new |
| `crates/rustamp-mediahost/src/pcm.rs` | 249 | modified (+directed entry points, `ClosePair`) |
| `crates/rustamp-mediahost/src/control.rs` | 315 | modified (`DspError` sticky-fail, `interrupt_reoffer`) |
| `crates/rustamp-mediahost/src/lib.rs` | 211 | modified (`pub mod dsp`, doc line) |
| `crates/rustamp-mediahost/src/bin/dsphost.rs` | 24 | rewritten (was 45-line ad-hoc loop) |
| `crates/rustamp-mediahost/examples/dsp_fixture.rs` | 24 | new |
| `crates/rustamp-mediahost/tests/dsp_named.rs` | 91 | new |
| `crates/rustamp-mediahost/tests/dsp_process.rs` | 106 | new |
| `crates/rustamp-mediahost/tests/support/dsp_resident.rs` | 98 | new |
| `Cargo.toml` (workspace members) | +1 | register `crates/rustamp-dsphost` |
| `Cargo.lock` | +10 | local `rustamp-dsphost` package entry only; no new external version |
| this card | new | owned doc path |

No file under `crates/rustamp-dechost`, `crates/rustamp-dsp`,
`crates/rustamp-core`, `crates/rustamp-ipc`, `crates/rustamp-audio`,
`crates/rustamp-app`, `crates/rustamp-ui` or `crates/rustamp-meta` was
touched.

## 7. Compile-only gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **exit 0** (compiles all new tests without running them).
- `cargo build --workspace`: **exit 0** (~48 s); both
  `target/debug/rustamp-dsphost.exe` (framework placeholder) and
  `target/debug/rustamp-dsphost-stages.exe` (real resident) link.
- **No `cargo test`** (not even `--no-run` binaries beyond what clippy
  compiles), no test process spawned, no example run, no window opened,
  no audio played, no download, no dependency addition beyond the four
  workspace-path edges above, no install, no commit, no git write. Only
  `cargo fmt`, `cargo clippy`, `cargo build` and read-only inspection
  commands were used.
- No Winamp source, SDK, disassembly or forbidden-tree contents were
  read. No third-party code copied. Worktree-local `git status --short`
  is the final boundary receipt below; it lists exactly the files above
  and nothing under `D:\code\Rustamp` (which was never accessed).
