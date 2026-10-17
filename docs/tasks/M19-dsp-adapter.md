# M19: Engine runs its DSP chain in the resident

## Boundary and scope

Write only `crates/rustamp-mediahost/src/**`, `crates/rustamp-dsphost/src/**`
(not needed in the end), `crates/rustamp-audio/src/{resident.rs,engine.rs}`
and this card in the assigned worktree. Nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written; no
dirty-room/reference material. Sources: RFCs 0003, 0006, 0007, the M13, M14,
M15 and M16 cards, the existing clean repository code only. No copied
third-party code, no downloads, no new dependencies, no commits, no windows,
no audio playback, no host test execution, no process spawns.

No `Cargo.toml`/`Cargo.lock` change at all: `rustamp-audio` already depends
on `rustamp-mediahost` and `rustamp-proto` (M15), and the adapter needs nothing
else — the stage list crosses as `rustamp_proto::host::DspStageDesc`, so the
GPL `rustamp-dsphost` resident crate stays out of the player's dependency
closure (RFC 0006 §8's one-way rule). `crates/rustamp-audio/src/lib.rs` is not
an owned path and was not touched: the new public items are reachable as
`rustamp_audio::resident::{HostDspChain, ProcessDspChain, DspResidentConfig,
DspFailureWatch, DEFAULT_DSP_RING_FRAMES, DEFAULT_DSP_BLOCK_TIMEOUT}` (the
`resident` module is public), the same way the decoder adapter's module is
reached before its root re-exports.

Not touched (other lanes): `crates/rustamp-dsp`, `crates/rustamp-core` (no
contract change — RFC 0014 stays a draft and is **not** implemented here),
`crates/rustamp-output`, `crates/rustamp-dechost`, `crates/rustamp-dsphost`,
every `tests/` directory (all new tests live inside `src/resident.rs`, like
M15's adapter tests), `xtask`, `docs/rfc/`.

## 1. The seam chosen: an adapter, not a second chain source

The engine runs its DSP chain in the resident by presenting the resident
session as the core `DspStage` the engine already drives — the DSP counterpart
of M15's decoder adapter, and the seam RFC 0006 §6.2 names ("the engine adds
the reported latency to its own compensation exactly as for
`DspStage::latency_frames`").

- `HostDspChain<F: PcmRings>` (`crates/rustamp-audio/src/resident.rs`) wraps a
  `DspSession<F>` and implements `rustamp_core::audio::DspStage`:
  `process` is one bounded block round-trip (write the block, read the same
  block back), `prepare` reconfigures on a format change and clears the
  resident's stage state otherwise, `reset` sends `DspReset` (seek), and
  `latency_frames` reports the session's `DspReady` latency.
- `ProcessDspChain` (Windows) owns a spawned `DspProcess` behind a mutex
  (poison-checked `get_mut`, mirroring `ProcessDecoder`) and delegates with the
  identical translations.
- `DspResidentConfig` names the helper binary explicitly and `open(spec, watch)`
  spawns it and returns the stage as `Box<dyn DspStage>`; installing it is the
  opt-in (`Engine::set_dsp_chain`).

Why this seam: it is the smallest honest one. `decode.rs` (not owned, not
changed) keeps driving the same `Vec<Box<dyn DspStage>>` chain — the gapless
loop, the seek drain, the format-change flush, the vis tap and the audio-clock
bookkeeping cannot diverge between in-process and resident stages because
there is only one code path, exactly M15's argument one level down. A second
chain source inside the decode thread would have duplicated every one of those
invariants and needed `decode.rs` edits outside this lane. The in-process chain
keeps working and stays selectable: the default engine chain is empty (plus
the per-run equalizer and limiter), no resident factory exists, and nothing
spawns unless the caller constructs a `DspResidentConfig` and installs the
result — never a silent default, because selecting the resident requires
naming a helper binary.

## 2. Latency compensation (RFC 0006 §6.2)

The adapter does no compensation of its own — it *reports* and the decode
thread's existing machinery compensates, which is why the reported frames stay
correct across a seek and a gapless seam with zero extra wiring:

- `HostDspChain::latency_frames()` returns `session.latency_frames()`, the
  number the resident reported in `DspReady`. The decode thread sums it into
  `dsp_latency()` exactly like the limiter's look-ahead: after every
  `prepare`/`reset` the first `latency` frames the chain emits are dropped
  (`dsp_trim`), and at the end of every track (and before a format change, and
  after a decoder error) `latency` frames of silence are pushed through the
  chain (`flush_dsp`) so the audio it still holds comes out.
- Because `process` is one synchronous round-trip, the block that comes back
  is the block that went in, delayed by exactly the chain's latency — the same
  shape as an in-process look-ahead stage. So per track: the first `latency`
  output frames (the chain's initial state) are trimmed at the start, and the
  last `latency` real frames are flushed out at the end. No frame is lost,
  none is duplicated, and no silence enters the seam.
- **Across a seek:** the decode thread calls `stage.reset()` (the adapter
  sends `DspReset`; the M13 pump services it between blocks and during its
  20 ms control-poll slices, so this also answers M14 F3's idle-reset
  concern), then re-arms `dsp_trim = dsp_latency()`. The resident's rings are
  empty at that point (the adapter drains fully on every block), so no stale
  audio survives the seek; the post-reset initial-state frames are trimmed and
  the engine re-bases the clock exactly on the target.
- **Across a gapless seam:** the track end flushes `latency` silence frames
  through the resident (emitting the track's tail), the next track's
  `prepare_track` calls `prepare` — same format, so the adapter sends
  `DspReset`, matching the in-process stages, which leave `prepare` with
  fresh state (`Limiter::prepare` calls `reset()`, `Equalizer::prepare`
  reallocates its history) — and re-arms the trim. The seam is sample-exact and
  the timeline continuous.
- **When the report disagrees with reality** (a lying or buggy resident
  stage): the compensation trims or flushes the wrong number of frames, so the
  error is bounded by the reported latency and shows up as a small position
  shift or a short tail at the track end — never a hang, a crash or a lost
  track. The report is trusted because RFC 0006 §6.2 makes `DspReady.
  latency_frames` the contract, exactly as an in-process stage's
  `latency_frames` is trusted. (M14 §4 lists "reported latency matches
  measured delay" as a VM check; the engine-level tests here pin the
  compensation with a synthetic 100-frame look-ahead, the real-binary
  measurement stays a VM item.)

## 3. Failures: refusal, death, wedge, reconfigure

The core `DspStage::process` contract is infallible (M14 finding F1; RFC 0014
is a draft and is **not** implemented here), and the decode thread discards
`prepare` errors (`let _ = stage.prepare(src)`), so a mid-run resident failure
cannot be returned per block. The adapter therefore latches: the first failure
sticks into a `DspFailureWatch` (an `Arc<Mutex<Option<ErrorInfo>>>` taken on
the decode worker and the engine's caller thread only — never the render
callback), the stage is poisoned and passes audio through **bit-exactly**
(undelayed, `latency_frames() == 0` so the trim cannot drop real audio), and
`Engine::poll` — new wiring in the owned `engine.rs` — takes the latch once and
fails the track via `handle_dsp_failure` (the `handle_output_error` teardown
minus the output-failure latch: stop the decode thread, pause/flush, freeze
the timeline at the failure position, publish `Event::Error` +
`StateChanged(Stopped)` + `PositionChanged`). RFC 0006 §8: the resident's
death fails the track, never the player.

- **Refusal at configure:** `DspResidentConfig::open` surfaces it before
  anything is installed — an unknown stage id / undecodable params become a
  loud `CoreError::Dsp` (the resident's `DspError` text rides along); a
  missing helper binary degrades to "stage unavailable"
  (`CoreError::Unsupported`), per RFC 0006 §8, never a silent in-process
  fallback. A refusal at a later *reconfigure* is a loud `Dsp` error from
  `prepare` **and** a latched failure, so the engine stops the track even
  though the decode thread ignores the `Err`.
- **Death mid-stream:** the framework's supervisor fails sticky on control
  EOF/missed `Pong`/`DspError` and its `ClosePair` abort wakes the blocked
  round-trip, so `process` returns within milliseconds; the session's
  drained-close path (a rare wake race that can read as EOS) is also latched —
  the adapter never treats `Ok(0)` mid-stream as good, it never calls
  `finish_writes`. The engine fails the track; a later run plays with the
  poisoned chain bypassing bit-exactly (pinned by test).
- **Wedge (alive but not pumping):** every blocking call is bounded. The
  adapter's round-trip uses the new `DspSession::write_timeout(buf, Some(..))`
  (framework addition in the owned `mediahost/src/dsp.rs`; `write` keeps its
  infinite signature for the existing tests) and the existing bounded `read`,
  both with the per-block `block_timeout` (`DEFAULT_DSP_BLOCK_TIMEOUT` = 5 s,
  configurable). A wedge therefore fails the block round-trip with `TimedOut`,
  latches, and the engine fails the track; the engine's `join_decode` is bounded
  by the same timeout, so nothing can hang. This also answers M14 L4's missing
  DSP bounded-wait coverage at the adapter level.
- **Reconfigure:** `prepare` with a new format calls `session.reconfigure`
  (both rings close and re-offer with bumped generations; the adapter is
  drained at that point, so no PCM dies with the old generation), and the new
  `DspReady` latency is picked up by the decode thread's next `dsp_latency()`
  re-arm. A refused reconfigure latches (above). Recovery from a poisoned
  chain is explicit: replace the stage (a fresh helper process), no auto-retry
  — RFC 0014 Q3's default, held adapter-side.
- **Drop:** `DspSession`'s drop closes both rings and sends `Shutdown`; the
  host exits `Ok`. One helper process per installed chain, for the chain's
  whole lifetime (RFC 0006 §6: one helper per DSP chain).

## 4. Tests (written, not executed here)

All in `crates/rustamp-audio/src/resident.rs` (the lane forbids touching any
`tests/` directory), headless against a **fake resident** — no process spawn
in any test: byte-stream control pairs exercise the production framing and the
real `NamedPcmRings` in one process, exactly like M13/M15. The fake resident
serves `gain`/`passthrough` plus test-only modes: `fake:delay` (a real
look-ahead: holds back N frames, reports N), `fake:slow` (wedges on its first
block), `fake:die` (panics on its third block — an in-process crash surrogate,
reaped through `catch_unwind`), `fake:refuse`, `fake:reconfig-refuse`. 18
tests:

Pure mapping/config (cross-platform, `mod dsp_tests`):

| Test | What it proves |
|---|---|
| `dsp_wire_spec_converts_and_rejects_wide_channels` | `AudioSpec` → wire spec; >255 channels refused, never truncated |
| `dsp_failures_are_loud_dsp_errors` | every `MediaHostError` variant maps to a loud `Dsp` error naming the resident |
| `dsp_failure_watch_latches_the_first_failure_once` | first failure sticks; `take` handles it exactly once |
| `dsp_config_defaults_and_overrides` | `DspResidentConfig` defaults and builders |
| `dsp_zero_ring_frames_is_rejected_loudly` | zero capacity refused before any spawn (both platforms) |
| `dsp_missing_helper_is_stage_unavailable` | missing binary → `Unsupported` ("stage unavailable"); other spawn failures → loud `Dsp` |

Adapter level (`#[cfg(all(test, windows))] mod dsp_hosted_tests`):

| Test | What it proves |
|---|---|
| `dsp_gain_round_trip_is_bit_exact_with_zero_latency` | exact ×2.0 per sample, `latency_frames() == 0`, `frames_dropped() == 0`, graceful drop ends the host `Ok` |
| `dsp_delay_chain_round_trip_is_the_delayed_stream_and_reset_clears_it` | the block comes back delayed by exactly the reported latency; `reset` clears the chain state and never touches the rings/generations |
| `dsp_prepare_reconfigures_on_a_format_change` | new spec → reconfigure, both generations bump, audio flows at the new format |
| `dsp_prepare_with_the_same_format_clears_state_without_reconfigure` | same-spec `prepare` sends `DspReset` (observable on the delay line) with generations unchanged |
| `dsp_refusal_at_prepare_is_loud_and_poisons_the_chain` | reconfigure refusal → loud `Dsp` error + latched watch; poisoned chain bypasses bit-exactly, reports latency 0, sticky error; host ends `Err` |
| `dsp_death_mid_stream_is_loud_and_poisons_the_chain` | two good blocks, the third dies with the host: buffer untouched, loud latched `Dsp` error, latency 0, host thread panicked (bounded reap) |
| `dsp_wedge_is_a_bounded_wait_and_poisons_the_chain` | 20 ms bound: the round-trip returns bounded (asserted < 1 s), buffer untouched, "timed out" latched, host survives and ends |

Engine level (same module; `Engine` + `NullOutput` + a pattern decoder
factory + `HostDspChain` installed via `set_dsp_chain` with
`engine.resident_dsp_watch()`; limiter disabled for bit-exact comparisons):

| Test | What it proves |
|---|---|
| `engine_compensates_the_resident_latency_exactly` | 100-frame look-ahead chain: the ring receives the track's 3000 frames bit-exact (trim at the start, flush at the end), run finishes, position freezes at 3000, `PlaylistEnded` |
| `engine_gapless_seam_through_the_resident_chain_is_exact` | two tracks through one resident chain: 6000 frames exact, no silence at the seam, both patterns in order, joint finish |
| `engine_seek_through_the_resident_chain_lands_exactly` | seek to 1000 lands exactly; post-seek recorded audio is the pattern at the settled position |
| `engine_fails_the_track_when_the_resident_dies` | mid-run crash → transport `Stopped` before the track end, `Event::Error` naming the resident, `last_error()` set; the player survives (a second run plays and finishes naturally) |
| `engine_fails_the_track_when_the_resident_wedges` | wedged resident → bounded stop with a "timed out" error event; the engine cannot hang |

### What needs the VM

- Everything above is written for the Hyper-V test VM and was **not executed
  on this host** (lane constraint). The Windows-gated tests run there like the
  rest of the suite; none is `#[ignore]`d (one-process, like M13/M15).
- Additionally requiring **two real processes** (therefore the VM, `#[ignore]`d
  pattern like `dsp_process`): spawning the real `rustamp-dsphost-stages`
  binary through `DspResidentConfig::open`/`ProcessDspChain` (bit-identity of a
  gain chain against in-process `rustamp-dsp` stages), an abrupt
  resident-process kill mid-stream via `ProcessDspChain::terminate()` (next
  `process` latches, `Engine::poll` fails the track), the missing-helper
  degradation (`Unsupported`), and the RFC 0006 §10.3 measurement "reported
  latency matches measured delay" with a real nonzero-latency stage. The
  `dsp_fixture` example + `dsp_process` template in `rustamp-mediahost` is the
  pattern; no new two-process test was added here because the adapter drives
  the identical wire in-process.
- Not covered here (named, not hidden): an engine-level mid-stream **format
  change** through the resident chain (covered at adapter level by
  `dsp_prepare_reconfigures_on_a_format_change`; the engine path resamples, so
  a bit-exact assertion would need a same-rate format change — a channel-count
  change test would be the follow-up), and `RUSTAMP_TESTDATA` media through
  the resident chain.

## 5. Own choices (everything not dictated by a spec or RFC)

1. Adapter over a second chain source (RFC 0006 §6.2 names it; `decode.rs`
   untouched, `set_dsp_chain`'s signature unchanged — RFC 0014 draft §3's
   constraint respected).
2. Opt-in by explicit construction: `DspResidentConfig::new(helper, stages)` +
   `open(spec, watch)` + `set_dsp_chain`. No registry, no default, no `PATH`
   lookup; the initial chain spec is an explicit `open` argument (the device
   spec is the natural choice) and the first differing `prepare` reconfigures.
3. Mid-run failures latch into an engine-polled `DspFailureWatch` and the
   stage bypasses bit-exactly; `Engine::poll` fails the track. This is the
   RFC 0012-style polled accessor that RFC 0014 §2 considered and rejected
   **for the core contract** — here it is adapter-local engine wiring (the
   only place the infallible `process` can be made loud without a core
   contract change), active only when a resident chain is installed.
4. Poisoned is terminal for the stage instance: first failure sticks, no
   auto-retry, recovery is an explicit replacement (RFC 0014 Q3's default).
5. All resident-DSP failures map to `CoreError::Dsp` with the resident named
   in the text (the decoder adapter's `Decode` analogue); a missing helper
   maps to `Unsupported` ("stage unavailable"). The core has no String-taking
   stage-unavailable variant, so the `Unsupported` message cannot name the
   helper path (the caller constructed the config and knows it).
6. `prepare` with an unchanged format sends `DspReset` — the decode thread
   calls `prepare` at every track start and the in-process stages leave
   `prepare` with fresh state, so the resident must too.
7. `DEFAULT_DSP_RING_FRAMES = 4096` (liveness/coupling, not latency: the
   round-trip is synchronous), `DEFAULT_DSP_BLOCK_TIMEOUT = 5 s` (per block
   round-trip; also bounds the engine's decode-thread join).
8. `Ok(0)` from a session read is always a loud failure for this adapter (it
   never finishes writes mid-stream), closing the drained-close wake race
   deterministically.
9. Stage `name()` is `"resident-dsp"`; `ProcessDspChain` exposes `pid()` and
   `terminate()` for VM crash-isolation tests; the watch lock is
   poison-tolerant (`into_inner`), never on the render callback.
10. `write_timeout` was added to `DspSession` (owned framework path) instead of
   bounding the write adapter-side: the ring handles are private to the
   session, and `write` keeps its infinite signature so the existing
   framework tests are untouched.
11. All tests live in `src/resident.rs` (the lane forbids `tests/`); the fake
   resident and engine harness are test-module-local and never shipped.
12. No `Cargo.toml`/`Cargo.lock` delta: the M15 dependency edges already cover
   the adapter; `rustamp-dsphost` stays out of the player's closure.

## 6. Files and line counts

Line counts via `Get-Content` (code + blanks, no diffstat).

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-audio/src/resident.rs` | 2454 | +1676/−3: DSP adapter section (`DspFailureWatch`, `DspResidentConfig`, `HostDspChain`, `ProcessDspChain`, mapping helpers) + 6 pure + 12 hosted/engine tests |
| `crates/rustamp-audio/src/engine.rs` | 1395 | +65: `dsp_watch` field, `resident_dsp_watch()`, the `poll` check, `handle_dsp_failure`, `set_dsp_chain` doc |
| `crates/rustamp-mediahost/src/dsp.rs` | 772 | +17/−4: `DspSession::write_timeout` (bounded write; `write` delegates) |
| this card | new | owned doc path |

No file under `crates/rustamp-dsp`, `crates/rustamp-core`, `crates/rustamp-output`,
`crates/rustamp-dechost`, `crates/rustamp-dsphost`, `crates/rustamp-audio/src/lib.rs`,
any `tests/` directory or `xtask` was touched. No `Cargo.toml`, `Cargo.lock` or
`THIRD_PARTY.md` change (no new dependency of any kind). The wire is unchanged:
no `Message`, ring-layout, generation or `PROTOCOL_VERSION` change.

## 7. Compile-only gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, includes the new pure, hosted and engine tests (compiled, not
  run). Three findings fixed on the way: `Error::Unsupported` takes a
  `&'static str` (message reworded), `clippy::mut_mutex_lock` (poison-checked
  `get_mut` instead of locking a `&mut Mutex`), `clippy::redundant_pattern_matching`
  in two test assertions.
- `cargo build --offline --workspace`: **exit 0**.
- **No `cargo test`** (not even `--no-run` beyond what Clippy compiles), no test
  process spawned, no example run, no window opened, no audio played, no
  download, no dependency addition, no install, no commit, no git write. Only
  `cargo fmt`, `cargo clippy`, `cargo build` and read-only inspection commands
  were used, all with `--offline` where the tool supports it.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read. No
  third-party code copied. Worktree-local `git status --short` is the final
  boundary receipt below; it lists exactly the files above and nothing under
  `D:\code\Rustamp` (which was never accessed).

```
$ git status --short
 M crates/rustamp-audio/src/engine.rs
 M crates/rustamp-audio/src/resident.rs
 M crates/rustamp-mediahost/src/dsp.rs
?? docs/tasks/M19-dsp-adapter.md
```
