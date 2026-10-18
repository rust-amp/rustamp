# W22: RFC 0006 remainder against the current mediahost

Status: implemented within the framework's ownership; VM acceptance pending.
Date: 2026-10-18. Lane: `wave102-mediahost`.

## Scope and provenance

Write only your own files, and nothing under `D:\code\Rustamp`.
Owned: `crates/rustamp-mediahost/**` and this card, in
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave102-mediahost`.
Initial worktree `git status --short`: empty. No other crate, contract,
manifest, lockfile or dependency was changed. No downloads, reference binaries,
Git writes/commits, tests, VM startup or broker invocation were performed.

Sources: AGENTS.md and its required project documents; accepted RFC 0006
(including the licensing erratum), RFC 0007's dated withdrawal, W19's root
card; the complete mediahost crate; read-only inspection of our dechost and
dsphost resident implementations, IPC ring/message implementation and audio
adapter call sites. No Winamp source, SDK material, disassembly or research
repository was read. No third-party code was copied.

## What the RFC requires, and what already exists

The ready-queue wording dates from an earlier implementation state. It must
not be read as authority to replace the framework with an embedded codec.

| Requirement | Current implementation, before this lane |
| --- | --- |
| §4: never drop decoder/DSP PCM; wait for data/space | Real. Blocking PCM calls, partial-write loops, named mapping/wake handshake, zero-drop assertions. `pcm.rs`, decoder/DSP workers and clients. |
| §5: one decoder, open info, generation replacement on seek/format, PCM, statistics, drained EOS, errors | Real worker/client in `decoder.rs`. One telemetry omission fixed below; EOS ordering qualification below. |
| §6: two opposite-direction rings, FIFO, configure, latency, reconfigure, reset | Real in `dsp.rs`. Reset is additionally fenced by RFC 0015's `DspResetAck`; resident processing is fallible per RFC 0014. One premature-close classification fixed below. |
| §7: wire types and compatibility rejection | Already in proto/IPC. Current protocol is **4**, not the RFC's historical proposed 2: hosting shipped as 3 after app v2; RFC 0015 added reset acknowledgement as 4. Do not decrement it. |
| §8: supervision and process isolation | Real `ControlPeer` reader/writer/monitor, sticky liveness failures and abort handles; `DecoderProcess`/`DspProcess` spawn without a console and kill/reap. Engine policy stays outside this crate. |
| §8 erratum: codecs/stages in separately licensed residents | Real `rustamp-dechost::RegistryResident`, binary `rustamp-dechost-symphonia`, uses the existing registry/Symphonia decoder. Real `rustamp-dsphost::StageResident`, binary `rustamp-dsphost-stages`, provides gain/passthrough. GPL/core/MPL dependencies stay out of this permissive framework. |
| §10: bit-identical reference decode and meaningful DSP acceptance | Existing resident suites include `real_decode` against repo `sine.flac`/direct decode and `dsp_roundtrip`; existing framework suites cover tiny rings, seek, format, errors, deaf wakes and process loss. Their existence is not a new test-run claim. |
| Engine wrapper | Read-only inspection finds existing `DecoderProcess`/`DspProcess` adapters in `rustamp-audio/src/resident.rs`; this is not an unimplemented framework responsibility. |

### What is genuinely still a stub

* The framework's `rustamp-dechost` and `rustamp-dsphost` binaries deliberately
  select `UnavailableResident`/`UnavailableDspResident`. Real binaries have the
  distinct names above. Linking a decoder into these placeholders would
  contradict the §8 erratum; no new codec or licensing boundary is invented.
* `lib.rs`'s old `MediaHost`/`DecoderHost`/`DspHost` single-message facade still
  returns `NotImplemented`. Neither serving loop uses it. It cannot represent
  the multi-message ring conversation. The crate docs now distinguish this
  legacy API from the real loops. Whether to retire that public facade is an
  API-maintenance question, not permission to silently break callers.
* The default `PcmRings` directed methods refuse `MainToHost`: that is an
  explicit capability boundary for decoder-only custom factories. Production
  `NamedPcmRings` overrides both methods; DSP transfer is not stubbed.
* Non-Windows cross-process mappings remain an explicit M6 refusal. The
  resident registry refuses URLs (M4); the wire/framework already transports
  them. Neither is a license to invent a network decoder in this crate.

## Implemented here

1. **Retire the stale edge-triggered explanation, not the defensive fallback.**
   `pcm.rs` claimed the ring still used RFC 0007 §6.2's edge refinement. W19
   removed all six guards, and that refinement/proof was **withdrawn**, not
   corrected. The comment now describes every-advance signalling and the
   retained 20 ms fallback for deliberately missing/mispaired wakes. A real
   peer crash does not close/signal on Drop; it still needs the independent
   monitor to close the supervised view. No wake algorithm or timing changed.
2. **Do not lose telemetry on a non-PCM decoder read.** The worker used to
   return on `Ok(0)` or `FormatChanged` before examining `stats()`. Skipped
   final packets or a new bitrate at that boundary disappeared forever or
   arrived only after another successful block. Compare/publish after every
   read, before the terminal/format marker. Unchanged tuples still emit no
   message; nothing becomes per-block telemetry when the tuple is unchanged.
3. **Do not call a premature DSP back-ring close EOS.** A ring can close
   before the independent control stream's `DspError` is observed. Before the
   player calls `finish_writes`, normal completion is impossible. The session
   now fails sticky as `Disconnected` with `before input finished` in the
   diagnostic instead of returning false `Ok(0)`. This is deliberately not
   presented as a complete solution to post-finish terminal ordering (below).

No sleeps were added. New ordering uses framed messages, published ring state
and channels; finite waits are loud stall escapes or existing event waits.

## New tests and their failure witnesses

All three are written but **not executed here**, per the lane restriction.
Clippy's all-target compilation checks their Rust code, not their assertions.

| Test | What forces the relevant order | Why the old implementation fails / mutation witness |
| --- | --- | --- |
| `final_packet_statistics_arrive_before_eos_without_another_pcm_block` (`pcm_transfer.rs`) | The resident changes its tuple only when its underlying read returns zero. Drain exactly all 4099 frames, then inspect stats at EOS. | Previously the EOS early return skipped publication: stats stays `(0, None)` instead of `(1, Some(64))`. Moving the compare back below the early return recreates the failure. Dropped/extra PCM also fails the exact frame-count assertion. |
| `format_boundary_statistics_arrive_before_format_changed_is_returned` (`pcm_transfer.rs`) | The same wrapper changes stats only on `FormatChanged`; the client drains exactly 513 old-format frames before accepting the new ring. Inspect stats immediately at the returned transition, before any new read. | Previously the format early return skipped publication. A later new-format block cannot update the client's cached tuple without another client read. Missing transition, wrong frame boundary, wrong channel count or stale stats all fail. |
| `back_ring_close_before_input_finished_is_sticky_failure_not_eos` (`dsp_control.rs`) | A scripted host completes the real two-ring handshake, closes its empty back-ring, then holds its healthy `ControlPeer` alive on a release channel. The test waits for the close publication before reading; it never finishes input. | Previously `read` returns `Ok(0)`, so `unwrap_err` fails. Removing the new guard recreates that deterministic failure. The exact diagnostic is checked on read and subsequent write; EOF/liveness failure cannot masquerade as success, and no assertion accepts a timeout. |

The normal drained/sticky-EOS tests remain unchanged and must still pass. No
test was made weaker or changed from failure to retry. Existing fixture sleeps
(`fixture:slow`/`fixture:exit`) were not introduced or relied on by these tests;
their old process-exit ordering is not a deterministic witness for this lane.

## Unsettled/out-of-ownership remainder: exact follow-up requests

1. **DSP completion after `finish_writes`: terminal evidence is missing.**
   After input is finished, an empty back-ring closed by failure still races
   `DspError`; ring state alone cannot distinguish failure from successful
   drain. Neither an arbitrary sleep/grace period nor treating a currently
   healthy monitor as proof solves that. Request an accepted protocol RFC:
   define a host-to-main DSP successful-completion marker (including its
   relationship to generation/reconfigure and errors), add it in
   `rustamp-ipc/src/message.rs` with a protocol bump/roundtrip/rejection tests,
   then require **both** marker and drained back-ring in `DspSession::read`.
   Alternative contract designs need the same unambiguous positive terminal
   evidence. This lane does not choose a wire shape or touch IPC/core.
2. **Decoder EOS wire order needs reconciliation.** RFC 0006 §5.6 literally
   says drain, send `DecoderEos`, then close. Current `serve_inner` closes,
   sends `DecoderEos`, then waits for seek/shutdown; `DecoderSession` drains
   before returning zero. RFC 0007 §6.5 explicitly describes close-then-drain
   as satisfying this acceptance. The observed `Decoder::read` semantics are
   safe, but the literal cross-channel order differs. Which wording is
   authoritative? If strict producer-side drain-before-marker is intended,
   IPC needs a lossless, cancellable `wait_drained` primitive; a full-ring
   `wait_writable` is not enough (it returns immediately on a partly full
   ring). Do not add a spin/sleep poll or silently amend the accepted RFC.
3. **Resident selection is policy, not an omitted codec implementation.**
   RFC 0006 §§2/9 deliberately do not choose the next codec/stage. A SID/Opus
   resident needs separately owned code/dependency/license work; URL support
   needs the M4 source/auth/buffering decisions. No new dependency or codec is
   needed for the already real Symphonia PCM transfer. If a caller still
   selects the generic unavailable binary instead of the named resident,
   fix discovery/packaging in the owning audio/app/xtask lane, not by linking
   that resident back into the permissive framework.
4. **Legacy facade:** decide removal/deprecation/compatibility separately;
   keep tests for real conversations distinct from its Hello/Pong surface
   tests. Real format-change coverage remains synthetic where no real codec
   fixture exercises a midstream transition; an actual codec fixture belongs
   in the resident/decoder lane and needs provenance.

## Host gates

All commands start with `Set-Location` to this worktree. Cargo is offline;
build output and TEMP/TMP are directed under this worktree's `target`.
Each native command's exit is captured immediately into a variable.

| Gate | Exit |
| --- | --- |
| Initial `cargo fmt --all -- --check` | 1; only four newly added assertions needed multiline formatting; corrected within owned files |
| Final `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** (42.36 s; includes compilation/checking of the new test targets) |
| `cargo build --workspace` | **0** (59.97 s) |

`cargo test` was **not** run; neither VM nor broker was invoked.

## VM acceptance, owned by the orchestrator

Run the mediahost suite (including these three assertions), resident
`real_decode`/`dsp_roundtrip`, engine adapter suites and the full workspace
suite via the **interactive-session** runner. Re-run ignored `pcm_process` and
`dsp_process` with staged fixture binaries and `ring-cross` in its owner lane.
Only those runs can prove actual execution, named-event/mapping behaviour
across processes, kill/EOF liveness, sample identity for real decoded files,
and compatibility with the concurrent audio/IPC/resident changes. Compilation
is not that proof. The post-finish DSP terminal ambiguity remains open even
if all existing tests pass.

## Files and boundary proof

| File | Final lines | Diff + / - |
| --- | ---: | ---: |
| `crates/rustamp-mediahost/src/lib.rs` | 221 | 8 / 3 |
| `crates/rustamp-mediahost/src/pcm.rs` | 357 | 7 / 6 |
| `crates/rustamp-mediahost/src/decoder.rs` | 830 | 12 / 9 |
| `crates/rustamp-mediahost/src/dsp.rs` | 903 | 8 / 3 |
| `crates/rustamp-mediahost/tests/pcm_transfer.rs` | 638 | 85 / 0 |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 667 | 63 / 0 |
| `docs/tasks/W22-mediahost-pcm.md` | 178 | 178 / 0 (new) |

Tracked source/test total: **+183 / -21** across six files; this new card is
additional. `git diff --check` is clean. Worktree status lists exactly these
six modified files plus this untracked card. Canonical
`git -C D:\code\Rustamp status --short`: **empty** after the gates. No other
crate, contract or dependency file was written and no Git index/commit action
was taken (`GIT_OPTIONAL_LOCKS=0` on the boundary checks).
