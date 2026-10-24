# W175 — The engine-side adapter for the out-of-process media host (RFC 0006 §5)

**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave175-mediahost-engine-adapter`.

## Boundary and scope

Owned paths: `crates/rustamp-mediahost/**`, `crates/rustamp-audio/**`, this
card, and one additive change to the VM test runner
(`docs/notes/vm-run-tests.ps1`, see "What needs the VM").

Nothing under `D:\code\Rustamp` was written; `D:\code\rustamp-research`, the
Winamp source, the SDK headers and any disassembly were never read. No
download, no window, no audio device, no git write, no commit.

Sources: RFC 0006 (§5, §8), RFC 0002, RFC 0003, RFC 0007, RFC 0013 and the
repository's own code and task cards (M10, M12, M14, M15, M17, M18, M24,
M25). No copied third-party code.

## What was already there

The adapter itself exists on `main` (card M15): `crates/rustamp-audio/src/
resident.rs` holds `HostDecoder`, `ProcessDecoder` and `ResidentFactory`, all
behind the core `rustamp_core::audio::Decoder` trait. This lane did **not**
invent a second decoder type and did **not** touch the engine's decode path
(`decode.rs`, `engine.rs`, `ring.rs`, `convert.rs` are unchanged).

What was missing is the **proof** and two small contract holes in the
supervision:

1. The **two-process path** was never exercised through the engine seam.
   M15 listed it as a VM check that was never written; `crates/rustamp-audio/
   tests/resident_process.rs` (new) is that check.
2. The engine's **supervision policy** was hardcoded to the framework's
   `Liveness::default()` (a 5 s wall-clock `Pong` trip wire — the flake class
   this repository's own guest proved, M24 §1.3). RFC 0006 §8 puts the
   supervision policy on the engine side, so it is now the engine's own
   configuration (`ResidentConfig::liveness`).
3. The **format negotiation** was an infallible cast from the wire spec to
   the core spec. It is now a validated, pure decision
   (`negotiated_spec`), and the adapter refuses a format it cannot adopt
   instead of building a decoder with a zero sample rate.
4. The **request mapping** (`MediaSource` → `MediaLocationWire`) was inline
   in the factory's `open`. It is now a pure function (`host_location`) with
   its own unit tests.

## RFC 0006 §5, item by item

| §5 item | State | Where |
|---|---|---|
| 1. `DecoderOpen { location, prefetch_frames }`; a `Reader` cannot cross the boundary; a non-UTF-8 path is refused | done (M15) + mapping extracted and unit-tested now | `host_location`, `ResidentFactory::open` |
| 2. `DecoderOpened { spec, duration_ms, seekable, metadata }` + the ring handshake | done (M15); the negotiation is now validated and pure | `negotiated_spec`, `HostDecoder::new` |
| 3. Decode loop; `DecoderStats { skipped_packets, bitrate_kbps }` on change only | done (M15) | `Decoder::skipped_packets`, `Decoder::stream_info` |
| 4. Seek: the host closes the ring, offers a fresh one, answers `DecoderSeeked` | done (M15) | `convert_seek` |
| 5. Format change: a new ring plus `DecoderFormat` | done (M15) | `convert_read` (`Error::FormatChanged { from, to }`) |
| 6. End of stream: drain, then `DecoderEos`; `Ok(0)` means "closed and consumed" | done (M15) | `DecoderSession::read` |
| 7. Failure: `DecoderError` → the engine surfaces it as an event exactly like a local decode error | **pinned at the engine level now** (the adapter level was pinned by M15) | `tests/resident.rs::a_resident_that_dies_mid_track_fails_the_track_loudly` |
| Engine side: one struct implementing `rustamp_core::audio::Decoder`; no second code path | done (M15) | `HostDecoder`, `ProcessDecoder` |
| §8 supervision: the engine spawns, runs `Hello`/`Ping` liveness, kills and reaps on close | done; the **policy is now the engine's own** | `ResidentConfig::liveness`, `DEFAULT_RESIDENT_LIVENESS` |
| §8 crash behaviour: a dead helper fails the track, never the player | done (M15) + pinned at the engine level now | `tests/resident.rs` |
| §8 restart policy ("retry once, then fail the track") | **not implemented — engine policy, not the contract.** The engine keeps the behaviour it has for local decoders (a failed track is reported and the run moves on). Changing it would touch the shared decode path, which this lane does not own. | — |
| §10.4 "the engine-side `Decoder` wrapper passes the existing engine tests unchanged" | done: the whole `rustamp-audio` suite is unchanged and green | — |

Out of scope per the RFC and left alone: 32-bit `dsp_` DLLs, the resampling
and buffering policy, RFC 0003 Q1 (the lossless mechanism is already in
place: the rings drop nothing, `frames_dropped() == 0` is pinned), the
late-joiner consumer, multi-tenancy, and URL transport (the resident refuses
URLs loudly; the resident crate owns that, not this lane).

## What was implemented in this lane

### `crates/rustamp-mediahost/src/control.rs`

`Liveness` gained `Debug, PartialEq, Eq` so the engine can carry the policy
in its own configuration and compare it in a test. No behaviour change.

### `crates/rustamp-audio/src/resident.rs`

* `DEFAULT_RESIDENT_LIVENESS` (100 ms interval, 30 s timeout) and
  `ResidentConfig::with_liveness`. The engine now supervises the helper with
  a **stall escape** instead of the framework's 5 s wall-clock trip wire. The
  reasoning is in the constant's documentation: a *dead* helper is caught by
  the control EOF at once; only a *wedged* one waits for the `Pong` deadline,
  and the per-`read` bound (`DEFAULT_READ_TIMEOUT`, 5 s) already fails that
  track much earlier. The DSP resident's policy is unchanged (that is §6, and
  the DSP helper owns the other side of it).
* `host_location(source)` — the pure request mapping. A `Reader` is refused
  with `Unsupported`, a non-UTF-8 path with a loud `Decode`.
* `negotiated_spec(spec)` — the pure format-negotiation decision. The
  negotiation is one-sided (the host owns the format); a format the wire
  cannot carry is refused loudly.
* `HostDecoder::new` and `ProcessDecoder::new` return `Result` and refuse a
  format the engine cannot adopt.
* `convert_read` / `convert_seek` go through the validated negotiation, so a
  host that announces a broken format is a loud protocol violation instead of
  a bogus `FormatChanged`.

### `crates/rustamp-audio/tests/resident.rs`

The fake resident learned a `die<seed>_<frames>.rtst` script: it serves the
pattern and then fails loudly, the way a resident that dies mid-track does.
The new test pins RFC 0006 §5.7 at the engine level: the failure arrives as
an `Event::Error` with `ErrorKind::Decode` that names the resident, and the
track ends — never as the end of the stream, never as silence, never a hang.

### `crates/rustamp-audio/tests/resident_process.rs` (new)

The **two-process** proof of the adapter: a helper in its own address space,
the real named PCM ring, the real control channel, the engine's own
`Decoder` seam in front of it. Headless and `#[ignore]`d (it spawns a
process), so it runs only in the Hyper-V test VM.

| Test | What it proves |
|---|---|
| `a_missing_helper_binary_degrades_to_an_unsupported_format` | §8: a missing helper is "format unsupported", never a crash. **Not ignored**: no process is started, so it runs everywhere. |
| `the_stub_resident_decodes_through_the_engine_adapter` | The handshake is replayed (format, length, seekability), the first frames carry the resident's pattern, the **seek handshake** runs for real (the host closes the ring and offers a fresh generation) and the pattern continues at the reached position, the telemetry (`skipped_packets`, bitrate) is replayed, every frame up to the end arrives and the end of the stream is **sticky**. |
| `a_helper_that_dies_mid_stream_is_a_loud_decode_error` | A helper that dies mid-stream is a loud decode error — never `Ok(0)`, never silence, and the blocked read is woken by the control EOF instead of hanging. |
| `the_real_resident_decodes_a_real_file_bit_exactly` | The **real** resident (`rustamp-dechost-symphonia`) decoding a real file: the handshake is replayed (format, length, the file's title) and the PCM that crossed the process boundary is **bit-identical** to the same decoder running in-process. |
| `the_real_resident_seeks_and_lands_on_the_same_audio` | The seek reaches the position the host reports, and the audio after the seek is the audio the same decoder produces in-process after the same seek. |
| `the_engine_plays_a_track_through_the_real_resident` | The **whole production path**: the registry holds the resident factory, the engine opens the track through the helper process and knows nothing about it, and the rendered audio is bit-identical to the in-process decode. |

The first three use the mediahost **stub** resident
(`RUSTAMP_DECODER_FIXTURE`, staged by the VM runner today); the last three
use the **real** resident (`rustamp-dechost-symphonia`). A test that needs the
real resident and does not find it prints a `SKIP` note and returns — it
never pretends to have run.

## What needs the VM

* The five ignored tests above. They are the only two-process proof of the
  adapter and they cannot run on the host by rule.
* The real resident has to be **staged** in the guest. `docs/notes/
  vm-run-tests.ps1` now builds `rustamp-dechost-symphonia` and stages it into
  `target\debug`, next to the test binaries; the test finds it there (or
  through `RUSTAMP_DECHOST_BINARY`, which overrides the search). This is the
  one change outside the owned crates, and it is additive.
* The stub resident is already staged (`RUSTAMP_DECODER_FIXTURE`), so the
  first three tests run in the guest without any further change.

## Own choices

1. The engine's liveness default is a stall escape (30 s), not the
   framework's 5 s. Reasoned in `DEFAULT_RESIDENT_LIVENESS`.
2. The liveness policy lives in `ResidentConfig` (the decoder seam). The DSP
   seam keeps the framework default: that is §6, its helper owns the other
   side of the handshake, and changing it here would be an unproven change.
3. The format negotiation is validated, not cast. The wire holds a `u8`
   channel count and a `u32` rate, both of which fit the core spec; a zero
   rate would divide the engine's clock by zero, so it is refused.
4. A `Reader` source is refused with `Unsupported` (as before) and a
   non-UTF-8 path with a loud `Decode` (as before): the wording of the
   non-UTF-8 refusal is left alone so the pinned behaviour does not move.
5. The two-process tests use the mediahost stub resident for the protocol
   cases and the real resident for the codec case, so that the protocol proof
   does not depend on a codec being staged and the codec proof is not faked.

## Gates

* `cargo fmt --all -- --check`: exit 0.
* `cargo clippy -p rustamp-mediahost -p rustamp-audio --all-targets
  --all-features -- -D warnings`: exit 0.
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0 (the workspace still compiles; no foreign crate needed a change).
* `cargo test -p rustamp-mediahost -p rustamp-audio --all-features -- --
  test-threads=1`: all green — `rustamp-audio` 86 unit + 3 + 55 + 13 + 1 + 7
  + 8 + 17 + 1 (`resident_process`) + 1 doc test, `rustamp-mediahost`
  2 + 3 + 9 + 3 + 2 + 11. Six tests are new, of which five are the VM-only
  two-process tests.
* `cargo test --workspace --all-features -- --test-threads=1`: green, no
  failure anywhere (the engine's own tests are unchanged, RFC 0006 §10.4).
* `docs/notes/vm-run-tests.ps1` parses (`Parser::ParseFile`, no errors).
* The five ignored tests were **not** executed here: they spawn a process and
  belong to the VM by rule. They are written to be run there, not faked.
