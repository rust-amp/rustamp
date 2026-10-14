# Task card A1 — Audio engine core (Wave 1)

**Goal**
`rustamp-audio` gets a working engine: decode thread → lock-free ring → output
callback, with transport commands, events, gapless transitions, volume/balance
and a raw visualization tap. `rustamp-output` gets `NullOutput`, a WASAPI
shared-mode device and a cpal fallback. When the card is done, a CLI example
plays a file from the command line.

**Lane** A — audio engine (`crates/rustamp-audio/`, `crates/rustamp-output/`,
RFC 0002 in `docs/rfc/`).

**Owned paths**
- `crates/rustamp-audio/**`
- `crates/rustamp-output/**`
- `docs/rfc/0002-*.md` (new)
- `crates/rustamp-core/src/audio.rs`, `crates/rustamp-core/src/error.rs`,
  `crates/rustamp-core/src/event.rs` — **only** the changes RFC 0002 describes.
- Do **not** edit `Cargo.toml` at the repository root. The workspace
  dependencies `rtrb`, `cpal` and `wasapi` are already prepared; crate-local
  additions go into the crate's own `Cargo.toml`.

**Allowed sources**
- `specs/S09-plugin-abi.md` (only for the DSP/vis data shapes, informational).
- `docs/rfc/0001-core-contracts.md` — the frozen contract this card builds on.
- The `rustamp-core` and `rustamp-decoders` crates in this repository.
- Public documentation of the crates used (`rtrb`, `cpal`, `wasapi`).
- `docs/PLAN.md` section 2 for the intended pipeline.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them, not even
  briefly. If a fact is missing, write a spec request instead of guessing.

**Spec dependencies**
- S07 (EQ) and S18 (output behaviors) do **not** exist yet and are **not**
  needed here. Do not implement EQ or out_ds-style fades/prebuffering; those
  come with M2.
- S08 (vis data derivation) does not exist yet. The tap fills `VisFrame` with
  the raw interleaved form only, as the contract describes.

**Interface contract**
- Uses `rustamp_core::Command`, `Event`, `EventBus`, `PlayerState`,
  `DecoderRegistry`/`Decoder`, `OutputDevice`/`RenderFn`, `DspStage`,
  `AudioSpec`, `VisFrame`.
- RFC 0002 (new file, bundle all three, do not split):
  1. `Decoder::skipped_packets() -> u64`, default `0`.
  2. `ReadSeek` requires `Send + Sync` (the decoder crate currently wraps its
     reader in a `Mutex`; note the follow-up for the `rustamp-decoders` lane).
  3. A signal for a format change in mid-stream (an `Error` variant or an
     `Event`, your choice — justify it in the RFC).
- Anything else that would change a frozen contract needs its own RFC.

**What the engine must do**
- Own a decode thread; the ring between decode thread and output callback is a
  lock-free SPSC ring (`rtrb`). The `RenderFn` runs on the output's audio
  thread: **no heap allocation, no locks, no blocking I/O, no logging** (see
  `CONTRIBUTING.md`). Underrun writes silence.
- Transport from `Command`: play, pause, stop, seek, volume, balance, next
  track. State changes are published as `Event`s on the `EventBus` and readable
  as a `PlayerState` snapshot.
- Position comes from `OutputDevice::played_frames()`, not from the decode
  side.
- Gapless: when the current track ends, the next one starts without a gap and
  without dropping the device. Sample-accurate join is a test.
- Volume `u8` 0..=255 and balance `i8` -127..=127 are applied by the engine
  (the ranges are provisional until S02/S07 exist; keep the mapping in one
  place and document it).
- Vis tap: a `VisFrame` filled with the raw 2×576 data, resampled/downmixed
  from the engine format. Derivation rules wait for S08.

**What the output crate must do**
- `NullOutput` — `OutputDevice` for tests, deterministic, with a way to drive
  time forward.
- `WasapiOutput` — WASAPI **shared** mode, `wasapi` crate, correct play/pause/
  flush/latency/played_frames and an event-driven render loop.
- `CpalOutput` — the cross-platform fallback behind the same trait.
- No allocation inside the render callback for any of them.

**What the example must do**
- `cargo run -p rustamp-audio --example play -- <file>` opens a file through
  `rustamp-decoders::default_registry()` (a dev-dependency of the crate),
  plays it until the end and exits cleanly. Optional flags `--null` (NullOutput)
  and `--seconds <n>` are welcome.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] transport state machine (play/pause/stop/seek/next) and the events it emits;
  - [ ] volume/balance mapping at the boundary values (0, 127/128, 255, -127, 0, 127);
  - [ ] gapless join: two tracks through `NullOutput`, the seam has no silence
        and no dropped frames, position is continuous;
  - [ ] seek accuracy through the engine (frame-count based, not time-based);
  - [ ] **no allocation on the render path**: a counting global allocator in a
        test binary that runs the `RenderFn` many times and asserts zero
        allocations, plus a soak run of at least a few seconds of virtual audio;
  - [ ] `VisFrame` is filled, and the tap introduces no allocation.
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` still green
      (it must — none of this touches `rustamp-plugin-abi`).
- [ ] `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] The example plays `crates/rustamp-decoders/tests/fixtures/sine.flac`
      without error (state it in the report; no fixture may be copied into
      `rustamp-audio`).

**Provenance notes**
- No third-party source is copied for this card; all crates used are ordinary
  dependencies. If any code is copied from a free project, stop and say so —
  it needs a `THIRD_PARTY.md` entry and the license header kept.

**Report back**
Which files changed, the RFC 0002 summary, the test list with results, and any
contract question that needs a new RFC.
