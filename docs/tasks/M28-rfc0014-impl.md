# M28: Implement RFC 0014 — fallible DSP stages

## Boundary and scope

Write only `crates/rustamp-core/**`, `crates/rustamp-audio/**`,
`crates/rustamp-mediahost/**`, `crates/rustamp-dsphost/**`,
`crates/rustamp-dsp/**`, this card, and — mandated by the task instruction
("implement the option the RFC recommends, exactly as written, **including
the error kind it proposes**", RFC 0014 §3.2) and **not** on the do-not-touch
list — the single file `crates/rustamp-proto/src/error.rs` (one appended enum
variant). Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written; no dirty-room/reference
material. Sources: RFC 0014 (accepted by the owner), RFCs 0001, 0002, 0006,
0012, cards M13/M14/M16/M19, and the existing clean repository code only. No
copied third-party code, no downloads, no new dependencies, no commits, no
windows, no audio playback, no host test execution, no process spawns.

`crates/rustamp-ipc`, `crates/rustamp-plughost`, `xtask` and the runner
scripts were **not** touched (verified by `git status --short` below:
`PROTOCOL_VERSION` lives in `rustamp-ipc` and is unchanged; no `Message`
diff exists — `ErrorKind`/`ErrorInfo` are not part of the ipc wire, they ride
the in-process event bus).

Note on line numbers: the RFC and the M16 card cite line numbers from an
older revision (`audio.rs:401` is now 423, `decode.rs:639` is now 680,
`mediahost/src/dsp.rs:39` is now 43, `eq.rs:429` is now 447, …). Every call
site was mapped by content; this card lists the current line numbers.

## 1. What was implemented (RFC 0014, Option A, exactly as written)

- **`rustamp-core`**: `DspStage::process(&mut self, buf) -> Result<()>` with
  the RFC's doc rule (`Err` is terminal for the run; the Ok path must not
  allocate). `Error::Dsp`'s doc widened from configuration-time to mid-run
  faults, and the classification maps `Error::Dsp(_) => ErrorKind::Dsp`
  instead of the `Other` fallthrough.
- **`rustamp-proto`**: `ErrorKind::Dsp` appended at the end of the enum (the
  RFC 0002 C4 maneuver — existing serialized indices do not move; a peer
  that never produces `Dsp` never emits the new index).
- **`rustamp-mediahost`**: `ResidentChain::process` gains `-> Result<()>`;
  `pump` propagates with `?` (`chain.process(..)?`). Nothing else in
  `serve_inner`/`serve_dsp` changes: the `Err` flows into the existing
  `DspError { message }` send and bounded flush, and the helper exits `Err`.
- **`rustamp-dsphost`**: `Chain::process` runs the stages in order and maps
  the first failure (`MediaHostError::OperationFailed(error.to_string())`),
  mirroring how `open` maps `prepare` failures. `Gain`/`Passthrough` keep
  their exact behaviour and return `Ok(())`.
- **`rustamp-audio` (engine)**: `emit` treats a stage error exactly like the
  decoder-error arm — `report_error`, flush the chain, end the track
  (`Outcome::Ended`). Because `emit`/`flush_dsp`/`on_format_changed` now
  return an `Emit` outcome (`Continue` / `Abandoned` / `StageFailed`) instead
  of `bool`, the three `track_loop` arms map it explicitly. A per-run
  `stage_failed` flag on the decode worker makes the first failure stick
  (RFC §4.2): a stage that keeps failing during the latency flush ends the
  track through the same arm without a second report. `set_dsp_chain` keeps
  its signature; its doc now states that `Err` ends the track.
- **`rustamp-dsp`**: `Equalizer` and `Limiter` return `Ok(())` with
  byte-identical success behaviour.

## 2. Call sites: what changed where

Legend: **behaviour** = observable behaviour changed; **compile** = only
compiles differently (`Ok(())` / `.unwrap()` / signature handling).

### Core contract (`rustamp-core`, `rustamp-proto`)

| Call site | Change | Kind |
|---|---|---|
| `core/src/audio.rs:423` (`DspStage::process`) | `-> Result<()>` + the RFC doc rule | **behaviour** (contract) |
| `proto/src/error.rs:11-25` (`ErrorKind`) | `Dsp` appended at the end | **behaviour** (new serializable kind; indices stable) |
| `core/src/error.rs:30-32` (`Error::Dsp`) | doc widened to mid-run faults | compile (doc) |
| `core/src/error.rs:66-72` (classification) | `Error::Dsp(_) => ErrorKind::Dsp` replaces the `Other` fallthrough | **behaviour** |
| `core/src/error.rs` (test) | new `dsp_faults_keep_their_kind_and_message` | test |
| `core/src/mock.rs:248` (`MockDsp::process`) | returns `Ok(())` | compile |
| `core/src/mock.rs:324` (test) | `.unwrap()` | compile |

### Engine chain (`rustamp-audio`)

| Call site | Change | Kind |
|---|---|---|
| `src/decode.rs:680` (`emit` stage loop) | the one production behaviour change: stage `Err` → `report_error` + `flush_dsp` + end the track (RFC §3.5); `emit`/`flush_dsp`/`on_format_changed` return `Emit` instead of `bool`; per-run `stage_failed` flag (first failure sticks) | **behaviour** |
| `src/decode.rs:1008` (`report_error`) | reused as-is | — |
| `src/engine.rs:366` (`set_dsp_chain`) | signature unchanged; doc note that stages run on the decode thread and `Err` ends the track | compile (doc) |
| `src/engine.rs` (latch wiring) | removed `dsp_watch` field, `resident_dsp_watch()`, the `poll` latch check and `handle_dsp_failure` | **behaviour** (see §3) |
| `tests/engine.rs:737` (`CountingDspStage`) | `Ok(())` | compile |
| `tests/engine.rs:1417` (`GainBoostDspStage`) | `Ok(())` | compile |
| `tests/engine.rs:1777` (`Poison`) | returns `Ok(())` — deliberate: the test pins the NaN sanitizer on the Ok path (RFC acceptance 4); the failure path is pinned by the new test below | compile (intent kept) |
| `tests/engine.rs` (new) | `FailingDspStage` + `a_stage_error_ends_the_track_loudly_and_only_once` | test (acceptance 2) |
| `tests/dsp_chain.rs:83,89` | `.unwrap()` | compile |
| `tests/limiter_gain.rs:59` | `.unwrap()` | compile |

### Resident chain (`rustamp-mediahost`, `rustamp-dsphost`)

| Call site | Change | Kind |
|---|---|---|
| `mediahost/src/dsp.rs:43` (trait) | `fn process` gains `-> Result<()>` + doc | **behaviour** (contract) |
| `mediahost/src/dsp.rs:195` (`pump`) | `chain.process(..)?` — the framework behaviour change: a mid-run chain error propagates into the existing `DspError` send + bounded flush; the helper exits `Err` | **behaviour** |
| `mediahost/tests/support/dsp_resident.rs:25` (`StubChain`) | `Ok(())` | compile |
| `mediahost/tests/dsp_control.rs:120` (`StatefulChain`) | `Ok(())` | compile |
| `mediahost/tests/dsp_control.rs` (new) | `FailingChain`/`FailingResident` + `midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text` | test (acceptance 3 + M14 F1 smallest reproduction) |
| `dsphost/src/lib.rs:67` (`Gain`) | `Ok(())` | compile |
| `dsphost/src/lib.rs:95` (`Passthrough`) | `Ok(())` | compile |
| `dsphost/src/lib.rs:158` (`Chain`) | iterate with `map_err(OperationFailed)` + `?` per RFC §3.4 | **behaviour** (error mapping only; success path identical) |
| `dsphost/src/lib.rs:296,306,326` (tests) | `.unwrap()` | compile |

### Stage implementations (`rustamp-dsp`)

| Call site | Change | Kind |
|---|---|---|
| `dsp/src/eq.rs:447` (`Equalizer`) | `Ok(())`; behaviour identical | compile |
| `dsp/src/eq.rs` tests (21 call sites) | `.unwrap()` on success paths | compile |
| `dsp/src/limiter.rs:536` (`Limiter`) | `Ok(())`; behaviour identical | compile |
| `dsp/src/limiter.rs` tests (12 call sites) | `.unwrap()` on success paths | compile |

### Adapter (`rustamp-audio/src/resident.rs`)

Not in the RFC §6 list (M19 postdates the RFC's call-site survey) but every
`DspStage` impl must follow the contract:

| Call site | Change | Kind |
|---|---|---|
| `HostDspChain`/`ProcessDspChain` `DspStage::process` (942, 1028) | now `CoreResult<()>`: a round-trip failure (death, wedge, drained close, lock poison) is returned as `Err(Error::Dsp(..))` with the text | **behaviour** |
| `ChainControl::poisoned` (739-781) | first failure sticks; every later `process` returns the sticky failure as `Err` (RFC §4.3); the buffer and the session are never touched again | **behaviour** |
| `ChainControl::prepare` (736) | signature unchanged; a refused reconfigure is returned from `prepare` **and** latched — the latched failure now surfaces from the next `process` (the decode thread discards `prepare`'s `Err`), so the engine hears it through the fallible channel | **behaviour** |
| `ChainControl::latency` (822) | poisoned → 0 (doc updated: the stage never processes again) | compile (doc) |
| `DspFailureWatch`, `Engine::resident_dsp_watch`, `Engine::poll` latch check, `Engine::handle_dsp_failure` | **removed** — see §3 | **behaviour** |
| `DspResidentConfig::open(spec, watch)` → `open(spec)`; `HostDspChain::new` / `ProcessDspChain::new` drop the `watch` parameter | public API change of the opt-in, unreleased adapter | **behaviour** (API) |
| `FakeChain`/`MultiChain` (tests), `open_chain`, `DspHarness`, engine-level tests | updated to the fallible contract; see §4 | test |

## 3. The adapter's latch: removed, no belt, no follow-up

The `DspFailureWatch` + `Engine::poll` polling + `handle_dsp_failure`
teardown (M19 §3) existed for exactly one reason: `DspStage::process` was
infallible, so a resident that died or wedged mid-stream had no per-block
channel — the RFC 0012-style polled accessor that RFC 0014 §2 considered and
rejected **for the core contract**. With a fallible `process`, the adapter
returns the error directly and the engine's existing `DecodeOut::Error` path
publishes it; the latch had no remaining work except one case, and that case
is not a workaround for the infallible `process`: the decode thread discards a
`prepare` error (`let _ = stage.prepare(src)`), so a refused reconfigure is
held in the stage-internal sticky `poisoned` state and returned from the next
`process`. That sticky state is stage-local (RFC §4.3's poisoned chain), not
an engine-polled latch.

Consequences, all intended:

- A poisoned chain no longer bypasses bit-exactly. Resuming on it is an
  error (RFC §4.3); recovery is an explicit replacement (`set_dsp_chain`
  with a fresh chain). Two M19 test claims changed accordingly (the "second
  run finishes naturally" halves now assert the loud sticky failure; the
  recovery half installs a fresh chain and finishes naturally).
- The M25 §1.1 / A25 erasure window is gone with the teardown: the failure
  now rides the decode thread's own FIFO report queue (`TrackStarted` is
  queued before the first block, the error after it), so the started track's
  duration is published deterministically before the error. The old
  `wait_latch` hold is deleted; `engine_publishes_the_start_when_the_resident_dies_right_after_it`
  pins the same claim without it.
- `Engine::poll` no longer stops the transport itself on a DSP failure; the
  transport leaves Playing through the normal end-of-run finish, exactly as
  it does for a decoder error (RFC §3.5: "exactly like the decoder-error
  arm").

Known boundary, documented not hidden: the one-`Event::Error`-per-run rule
(RFC §4.2) is enforced **for the stage-failure path** (the `stage_failed`
flag), which is what acceptance 2 pins. A decode error followed by a stage
error during that flush can still publish two events — the same shape as two
decode errors in one run today; the decode-error path is untouched by this
RFC. And at the framework level, `DspSession::read`'s drained-close path
still carries the M19 §3 wake race (a read blocked on an *empty* ring exactly
at the failure can observe `Ok(0)` before the `DspError` lands, because the
host closes its rings before sending it); RFC 0014 §3.3 leaves `DspSession`
untouched, and the acceptance-3 test pins "never `Ok(0)`" in the backlog
shape where the framework guarantees it (see §4).

## 4. Tests (written, NOT executed here)

| Test | What it proves |
|---|---|
| `core/src/error.rs`: `dsp_faults_keep_their_kind_and_message` | a stage fault classifies as `ErrorKind::Dsp` (not `Other`) and its text survives the `ErrorInfo` conversion |
| `mediahost/tests/dsp_control.rs`: `midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text` | **M14 F1's smallest reproduction** (acceptance 3, 6): a `ResidentChain` failing after block 1024 with `"clipping detected"` compiles and its text reaches the player. A 1024-frame healthy backlog is written first (one frame per block; the in-ring holds one frame) and drains bit-exact across the failure; the terminal read fails loud `Disconnected("resident DSP failed: … clipping detected")`, never `Ok(0)`; the failure sticks; the host exits `Err` carrying the text. The backlog shape is what makes "never `Ok(0)`" deterministic: drain-then-close hands the healthy frames over first, and the `DspError` lands within microseconds while ~1024 one-frame reads drain — the drained-close wake race (M19 §3) is only reachable by a read blocked on an empty ring exactly at the failure |
| `audio/tests/engine.rs`: `a_stage_error_ends_the_track_loudly_and_only_once` | **acceptance 2**: a `FailingDspStage` returning `Err(Error::Dsp("clipping detected"))` after block 1, reporting a 64-frame look-ahead so the end-of-track flush re-runs the failing stage: the track ends before its full length, the transport leaves Playing, `Event::Error` carries `ErrorKind::Dsp` + the message, `last_error` is set, and **exactly one** error event is published (the flush re-failure does not re-report) |
| `audio/src/resident.rs` (Windows-gated, in-process fake resident): `dsp_refusal_at_prepare_is_loud_and_poisons_the_chain`, `dsp_death_mid_stream_is_loud_and_poisons_the_chain`, `dsp_wedge_is_a_bounded_wait_and_poisons_the_chain` | the adapter returns the loud sticky `Err` from `process` (message intact, buffer untouched, latency 0, session never touched again); the refused reconfigure still ends the host `Err`; the wedge stays a bounded wait |
| same module: `engine_fails_the_track_when_the_resident_dies`, `engine_fails_the_track_when_the_resident_wedges`, `engine_publishes_the_start_when_the_resident_dies_right_after_it` | the engine-level behaviour on the new path: the decode thread reports the failure and ends the track, the transport stops, one `Event::Error` with `ErrorKind::Dsp` per run; resuming on the poisoned chain fails loudly (RFC §4.3); a **fresh chain** recovers and plays to the end; the started track's duration is published before the error (M25 §1.1 closed structurally) |
| same module: `dsp_gain_round_trip_is_bit_exact_with_zero_latency`, `dsp_delay_chain_round_trip…`, `dsp_prepare_reconfigures…`, `dsp_prepare_with_the_same_format…` | healthy chain unaffected: only `.unwrap()` added on the success paths |
| `dsp/src/eq.rs`, `dsp/src/limiter.rs`, `dsphost/src/lib.rs`, `core/src/mock.rs`, `audio/tests/dsp_chain.rs`, `audio/tests/limiter_gain.rs` | existing suites keep every assertion with only `Result`-handling edits — acceptance 1's "byte-identical behaviour on success paths" is a test-run claim for the VM |
| `Poison` (`audio/tests/engine.rs`) | still returns `Ok(())` after emitting NaN/∞ — acceptance 4's regression test: the `0.0` sanitizer still catches a NaN-emitting **healthy** stage; an `Err` bypasses sample post-processing entirely (structural: `emit` returns before the sanitizer) |

## 5. RFC §8 open questions — resolved per the RFC's own recommendations

| ID | Choice made | Where |
|---|---|---|
| Q1 stop vs bypass | **Stop** (the RFC's recommendation): `emit` ends the track exactly like the decoder-error arm. No bypass, no auto-recovery, no UI policy (RFC §5) | `decode.rs` `emit` |
| Q2 `ErrorKind::Dsp` vs reuse `Other` | **Append `Dsp`** — RFC §3.2 as written; the task instruction says to implement the error kind the RFC proposes. `rustamp-proto/src/error.rs` is outside this card's owned-path list but is not on the do-not-touch list and the RFC mandates it — flagged for the orchestrator | `proto/src/error.rs`, `core/src/error.rs` |
| Q3 resident restart policy | **Explicit caller action, no auto-retry** (the RFC's default): no session convenience re-open; the poisoned chain errors on resume; recovery is `set_dsp_chain` with a fresh chain — pinned by `engine_fails_the_track_when_the_resident_dies` | `resident.rs`, tests |
| Q4 out-of-tree implementers | Whole-tree search (`fn process(`, `.process(`, `impl DspStage for`, `impl ResidentChain for`) confirms every implementer and every ignoring caller is in-tree; the breaking Option A needed no `try_process` fallback | this card §2 |

Also for the orchestrator: RFC 0014's status line still reads **Proposed** —
`docs/rfc/` is not an owned path of this lane; the owner accepted the RFC, so
the integration lead should flip the status to Accepted.

## 6. What only the VM can prove

- **The entire test suite.** Every new and changed test is compiled here
  (clippy `--all-targets`) but not run (lane constraint). The Windows-gated
  tests (`dsp_control.rs`, `resident.rs`'s `dsp_hosted_tests`,
  `dsp_roundtrip.rs`) run in the Hyper-V test VM like the rest of the suite;
  none is `#[ignore]`d.
- The timing margins the new tests rely on under a loaded VM schedule: the
  1024-frame backlog drain vs the `DspError` landing (`dsp_control`), and
  the engine-level failure flows (death/wedge/poisoned-resume/fresh-chain).
- Acceptance 1's "existing EQ/limiter/gain/passthrough tests pass with only
  `Result`-handling edits" — a test-run claim, VM only.
- The two-process variants (VM-gated `#[ignore]` pattern, unchanged by this
  lane): spawning the real `rustamp-dsphost-stages` helper through
  `DspResidentConfig::open`/`ProcessDspChain`, an abrupt resident kill via
  `ProcessDspChain::terminate()`, the missing-helper degradation, and the
  RFC 0006 §10.3 "reported latency matches measured delay" measurement.
- `RUSTAMP_TESTDATA` media/golden runs — untouched paths.
- Real exit codes of the helper binaries on a mid-run chain failure.

## 7. Files and line counts

Line counts via `Get-Content | Measure-Object -Line` (content lines, no
diffstat). Diffstat: 16 files, +603/−438.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-core/src/audio.rs` | 465 | `DspStage::process` fallible + doc rule |
| `crates/rustamp-core/src/error.rs` | 101 | `Error::Dsp` doc, `ErrorKind::Dsp` mapping, new unit test |
| `crates/rustamp-core/src/mock.rs` | 299 | `MockDsp::process` returns `Ok(())`; test `.unwrap()` |
| `crates/rustamp-proto/src/error.rs` | 34 | `ErrorKind::Dsp` appended (outside the owned list, mandated — see §5 Q2) |
| `crates/rustamp-mediahost/src/dsp.rs` | 778 | `ResidentChain::process` fallible; `pump` propagates with `?` |
| `crates/rustamp-mediahost/tests/support/dsp_resident.rs` | 91 | `StubChain::process` returns `Ok(())` |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 344 | `StatefulChain` `Ok(())`; new mid-run failure test |
| `crates/rustamp-dsphost/src/lib.rs` | 316 | `Gain`/`Passthrough` `Ok(())`; `Chain` maps the first failure; crate doc; tests `.unwrap()` |
| `crates/rustamp-dsp/src/eq.rs` | 838 | `Equalizer::process` returns `Ok(())`; 21 test call sites `.unwrap()` |
| `crates/rustamp-dsp/src/limiter.rs` | 1154 | `Limiter::process` returns `Ok(())`; 12 test call sites `.unwrap()` |
| `crates/rustamp-audio/src/decode.rs` | 987 | `Emit` outcome, `stage_failed` flag, the RFC §3.5 arm, module doc |
| `crates/rustamp-audio/src/engine.rs` | 1357 | latch wiring removed; `set_dsp_chain` doc |
| `crates/rustamp-audio/src/resident.rs` | 2736 | fallible adapter, `DspFailureWatch` removed, poisoned-chain semantics, tests rewritten |
| `crates/rustamp-audio/tests/engine.rs` | 2300 | three stage doubles `Ok(())`; new `FailingDspStage` + acceptance-2 test |
| `crates/rustamp-audio/tests/dsp_chain.rs` | 160 | two `.unwrap()` |
| `crates/rustamp-audio/tests/limiter_gain.rs` | 83 | one `.unwrap()` |
| this card | new | owned doc path |

No `Cargo.toml`/`Cargo.lock`/`THIRD_PARTY.md` change (no new dependency of
any kind). No file under `crates/rustamp-ipc`, `crates/rustamp-plughost`,
`xtask`, any runner script, `docs/rfc/` or another lane's crate was touched.
The wire is unchanged: no `Message`, ring-layout, generation or
`PROTOCOL_VERSION` change.

## 8. Gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass; 12 formatting diffs
  from the first check were fixed by hand, then re-verified).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0** — includes all new and rewritten tests (compiled, not run). One
  finding fixed on the way: an unused `buf` parameter in the new
  `FailingChain` (`_buf`).
- `cargo build --offline --workspace`: **exit 0**.
- **No `cargo test`** (not even `--no-run` beyond what Clippy compiles), no
  test process spawned, no example run, no window opened, no audio played,
  no download, no `cargo add`, no dependency addition, no install, no
  commit, no git write. Only `cargo fmt --check`, `cargo clippy` and
  `cargo build` (all `--offline`) plus read-only inspection were used.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read.
  No third-party code copied. Clean-room provenance: RFC 0014 (accepted),
  RFCs 0001/0002/0006/0012, cards M13/M14/M16/M19, in-tree code only.
- M14 finding **F1 is closed**: the smallest reproduction (a `ResidentChain`
  failing after block *n* with `"clipping detected"`) compiles, and its text
  reaches the player — pinned by
  `midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text` (test-run
  claim: VM).

## 9. Boundary receipt

`git status --short` inside this worktree (proof elicited after writing this
card) lists exactly the 16 files above plus this card, and nothing else:

```
 M crates/rustamp-audio/src/decode.rs
 M crates/rustamp-audio/src/engine.rs
 M crates/rustamp-audio/src/resident.rs
 M crates/rustamp-audio/tests/dsp_chain.rs
 M crates/rustamp-audio/tests/engine.rs
 M crates/rustamp-audio/tests/limiter_gain.rs
 M crates/rustamp-core/src/audio.rs
 M crates/rustamp-core/src/error.rs
 M crates/rustamp-core/src/mock.rs
 M crates/rustamp-dsp/src/eq.rs
 M crates/rustamp-dsp/src/limiter.rs
 M crates/rustamp-dsphost/src/lib.rs
 M crates/rustamp-mediahost/src/dsp.rs
 M crates/rustamp-mediahost/tests/dsp_control.rs
 M crates/rustamp-mediahost/tests/support/dsp_resident.rs
 M crates/rustamp-proto/src/error.rs
?? docs/tasks/M28-rfc0014-impl.md
```

`D:\code\Rustamp`, `D:\code\rustamp-research` and `D:\code\rustamp-refbin`
were never accessed (read or written) by this lane.

## 10. Merge record (2026-10-18, wave 83)

This commit was rebased onto `main` for integration (linear history: one
commit on top of `main`, fast-forwardable). The lane's original no-commit
boundary is superseded by the orchestrator's merge instruction — a commit
inside this worktree is expected — and every write stayed inside it. When
the lane finished, `main` had advanced fourteen commits past this card's
base (`bb4cdec`): the RFC 0015 DSP reset fence (`408863c`), the
plugin-fixture staging report (`4335293`), the smoke-capture fix
(`6abaa52`), the release plan (`ebae8b5`), the manifest-order fix
(`f8e5664`), the tray/hotkey seams (`d2c8e5d`), the Paseo routing rule
(`3b5309b`), the DSP plugin acceptance proof (`e227b37`), the
load-playlist contract and the `0.1.0-beta.1` version bump (`21f10b9`),
the M30 erasure twins (`1e8c0fe`), the wall-clock sweep (`c9217a6`), the
Milkdrop plan (`d166c92`), the visualiser (`a2106ed`) and — landing while
this merge was being resolved — the tray-gap close (`8f946a9`). The last
of those touches only `rustamp-app/src/window.rs`,
`rustamp-platform/{lib,tray}.rs` and `docs/tasks/F5-tray-seams.md`, none
of them in this lane's diff, so it rebased in without a conflict.

### Conflicts and resolutions (both sides kept everywhere)

| File | Conflict | Resolution |
|---|---|---|
| `crates/rustamp-mediahost/src/dsp.rs` | none textually — disjoint hunks, auto-merged | RFC 0014's fallible `ResidentChain::process` and `chain.process(..)?` in `pump` coexist with RFC 0015's `DspResetAck` fence at every poll site. No semantic interaction: the fence only touches `reset` (signature unchanged), and a chain failure exits `pump` via `?` into the existing `DspError` path before any further reset handling. |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | both sides inserted tests at the same anchor (after `idle_reset_lands…`) | Both kept: main's two RFC 0015 fence tests (`queued_resets_are_each_acknowledged_in_order_while_the_host_is_busy`, `reset_without_an_acknowledgement_is_a_loud_bounded_timeout`) first, then this lane's `midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text`. |

### Coherence adjustments beyond the textual merge (nothing dropped)

- `audio/src/engine.rs`, `join_decode` doc: main's `1e8c0fe` named
  `handle_dsp_failure`, which this lane deleted; the doc now names only
  `handle_output_error` (which `1e8c0fe` taught to deliver the held start)
  and records that a DSP-stage failure needs no delivery, because RFC 0014
  queues `TrackStarted` ahead of the error on the decode thread's own
  report queue.
- `audio/tests/engine.rs`: this lane's new test's single wait was adapted
  from the frame-budget `wait_until` (deprecated by `c9217a6`'s sweep) to
  the merged suite's `wait_pumping(…, WAIT)` convention; the asserted
  condition (`transport() == Stopped`) is unchanged. It was the only
  `wait_until` call left in the tree.
- Whole-tree check on the merged tree: no code outside this lane's files
  implements `DspStage`/`ResidentChain` or calls the changed adapter API
  (`DspResidentConfig::open`, `HostDspChain::new`, `ProcessDspChain::new`,
  `resident_dsp_watch`); `emit`/`flush_dsp`/`on_format_changed` are private
  to `decode.rs`; the mediahost examples reuse `StubChain` through a
  `#[path]` include and compile against the fallible trait unchanged.

### Stale-claim notes (this lane's own diff is unchanged)

§1/§7's "the wire is unchanged — no `Message`, ring-layout, generation or
`PROTOCOL_VERSION` change" describes this lane's diff and stays true (the
lane touched no `rustamp-ipc` file). Main has since bumped
`PROTOCOL_VERSION` 3 → 4 and added `Message::DspResetAck` (RFC 0015); both
survive in the merged tree. §2's line numbers refer to the pre-merge tree:
`mediahost/src/dsp.rs` is now 833 lines (the trait's `process` ~49, the
`pump` call ~217), `dsp_control.rs` 567, `engine.rs` 1642 (main's `1e8c0fe`
added its test module), `tests/engine.rs` 2271 (the `c9217a6` sweep).

### Post-merge gates (host, offline; the suite itself stays VM-only)

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo build --offline --workspace`: exit 0.
- Still no `cargo test`, no VM, no download, no new dependency, no window,
  no audio. `D:\code\Rustamp`, `D:\code\rustamp-research` and
  `D:\code\rustamp-refbin` were never accessed; every write stayed inside
  this worktree.
