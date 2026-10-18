# M26 — Refute the parallel-rings name collision; green the moving reds at the remaining wall-clock bounds

**Date** 2026-10-18. **Lane** `wave75-parallel-rings`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave75-parallel-rings`.

Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary. No test process, example,
player, helper binary or reference binary was spawned; no VM was touched; no
window or audio device was opened; no download, no dependency addition, no
install, no commit and no other git write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §7). Every command ran
inside this worktree.

The r45 evidence, five failures across three binaries:
`real_file_opens_with_format_handshake_and_decodes_bit_exact` and
`end_of_stream_comes_only_after_the_ring_is_drained`
(`crates/rustamp-dechost/tests/real_decode.rs`, both panicking at
`real_decode.rs:155`, "received no PCM within 30s … without EOS or an error"),
`resident_to_inprocess_source_change_is_gapless`
(`crates/rustamp-audio/tests/resident.rs:473`, `assertion failed: h.wait_track(0)`)
and two tests in `crates/rustamp-audio/src/resident.rs` with the same
`assertion failed: h.wait_track(0)`. r44 failed different tests in the same
binaries; one r44 red passed in r45 and the reverse. The prime suspect handed
to this lane: two parallel tests in one binary constructing the same **named**
ring/control object, so one test's host answers on the other test's name.

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-mediahost/**`, `crates/rustamp-dechost/**`,
`crates/rustamp-dsphost/**`, `crates/rustamp-audio/**`, and this card.
Touched (all test surface; no production code):

- `crates/rustamp-audio/tests/resident.rs`
- `crates/rustamp-audio/src/resident.rs` (test modules only)
- `crates/rustamp-mediahost/tests/{pcm_transfer,dsp_control,pcm_process,dsp_process}.rs`

Read only (not modified): `crates/rustamp-mediahost/src/{pcm.rs,control.rs,decoder.rs,dsp.rs}`,
`crates/rustamp-ipc/src/shm/{region.rs,wake.rs,ring.rs}`,
`crates/rustamp-dechost/src/lib.rs`, `crates/rustamp-dechost/tests/real_decode.rs`,
`crates/rustamp-dsphost/tests/dsp_roundtrip.rs`,
`crates/rustamp-decoders/src/**`, `crates/rustamp-audio/tests/{common/mod.rs,engine.rs,flush_contract.rs,render_rt.rs}`,
`crates/rustamp-audio/src/resident.rs` production sections, and the cards
M10, M20, M22, M24, M25, `docs/HANDOFF.md`.

Sources: the task cards named above, RFCs 0003/0006/0007 (as cited by them),
and the existing clean repository code only. No Winamp source, SDK material,
disassembly, research tree or reference binary was read; no third-party code
was copied; no dependency was added.

---

## 1. The prime suspect: REFUTED — the names cannot collide

### 1.1 The naming rule, with file:line

Every ring the failing tests construct goes through one of two factories:

1. **`NamedPcmRings::create_impl`** — `crates/rustamp-mediahost/src/pcm.rs:153-168`:

   ```rust
   static SERIAL: AtomicU64 = AtomicU64::new(1);            // pcm.rs:153, per process
   let name = format!(
       "{}-{:x}",
       ring_name(RingKind::Pcm, SERIAL.fetch_add(1, Ordering::Relaxed)),  // pcm.rs:163
       SystemTime::now()…as_nanos()                                     // pcm.rs:164-167
   );
   ```

   and `ring_name` — `crates/rustamp-ipc/src/shm/region.rs:267-274`:

   ```rust
   format!("rustamp-{}-{}-{:016x}", kind.name(), std::process::id(), generation)
   ```

   So the full section name is **`rustamp-pcm-<pid>-<serial:016x>-<nanos:x>`**.
   `SERIAL` is a single per-process `AtomicU64` shared by *every* construction
   in the process and monotonic (`fetch_add`), so each `create` call in a
   process gets a distinct serial; the pid inside `ring_name` separates
   processes (including helper children, which get their own pid). Two
   constructions therefore never share a name — not within one test binary
   (parallel tests share the one monotonic counter) and not across processes.

2. **`TestRings`** (the anonymous-region factory used only by
   `crates/rustamp-mediahost/tests/pcm_transfer.rs:93-146`): name =
   `mediahost-test-<pid>-<NEXT>` with `NEXT` a per-process monotonic
   `AtomicU64` (`pcm_transfer.rs:98-107`) — unique per construction, same rule.

Wake events derive from the ring name: `ring_wake_name` —
`crates/rustamp-ipc/src/shm/wake.rs:54-58` produces `<ring name>-data` /
`<ring name>-space`, so they inherit the uniqueness. `RingWake::create`
(`wake.rs:336-338`) and `NamedRegion::create` (`region.rs:141-176`) use
`CreateEventW`/`CreateFileMappingW`, which return the *existing* object when
the name already exists (`region.rs:136-140` documents exactly that for
sections) — but a test-path name never repeats, so that branch is unreachable
from the test surface.

Every test that constructs rings, and the factory it uses:

| Test surface | Factory | Name rule |
|---|---|---|
| `rustamp-dechost/tests/real_decode.rs` (`start()`, `DecoderSession::open`) | `NamedPcmRings` | §1.1 rule 1 |
| `rustamp-audio/tests/resident.rs` (`FakeResidentFactory::open`) | `NamedPcmRings` | §1.1 rule 1 |
| `rustamp-audio/src/resident.rs` `hosted_tests` / `dsp_hosted_tests` | `NamedPcmRings` | §1.1 rule 1 |
| `rustamp-dsphost/tests/dsp_roundtrip.rs` (incl. the two abrupt-loss tests' direct `create_directed`) | `NamedPcmRings` | §1.1 rule 1 |
| `rustamp-mediahost/tests/{pcm_named,dsp_named,dsp_control,pcm_process,dsp_process}.rs` | `NamedPcmRings` | §1.1 rule 1 |
| `rustamp-mediahost/tests/pcm_transfer.rs` | `TestRings` | §1.1 rule 2 |
| `rustamp-mediahost/examples/{decoder_fixture,dsp_fixture}.rs` (fixture children) | `NamedPcmRings` | §1.1 rule 1, child pid |

The control channel in all of these tests is **in-process `mpsc` byte
streams**, not a named kernel object — the "named control channel" half of the
hypothesis has no object that could collide at all.

**Verdict: the suspected collision cannot happen.** Two parallel tests in one
binary never construct the same named object, because the per-process serial
counter they *share* is exactly what makes each name unique; and the
"resident stops delivering while another test's host answers on the same
name" symptom has no mechanism. The moving failures are not name collisions.

### 1.2 The reverse question: resources that should be unique but are not

Swept the whole owned test surface for the reverse hazard (a temp path, a
fixed port, a shared file, a static counter shared across threads):

- **Ring/section/wake names:** unique per construction (§1.1). No collision.
- **Temp paths:** `real_decode.rs:180-185` uses pid + a per-process monotonic
  counter (`rustamp-dechost-<pid>-<n>-<name>`); the audio harnesses use
  `tempfile::tempdir()` (unique per call); `tests/resident.rs:391` `NEXT_ID`
  only decorates refuse-stem names. Unique.
- **Ports:** none — all control channels are in-process byte streams;
  production uses pipes.
- **Shared files:** fixtures are read-only; `default_registry()` is fresh per
  call (`rustamp-decoders/src/lib.rs`); `grep` over `rustamp-decoders/src`
  finds no `static`/`Mutex`/`OnceLock`/`lazy` — no shared decoder state.
- **Static counters:** `SERIAL` (pcm.rs:153), `NEXT` (pcm_transfer.rs:98),
  `NEXT` (real_decode.rs:180), `NEXT_ID` (tests/resident.rs:391) are shared
  across threads but monotonic — sharing them is the mechanism that makes the
  derived names unique, not a hazard.
- **Named observation (intentional, not a hazard):** `crates/rustamp-audio/tests/render_rt.rs:25-59`
  installs a process-global `CountingAllocator` (`static ALLOCATOR`,
  `static ALLOCATIONS`) — a shared static across parallel tests by design;
  the file header states "There is exactly one test in this file on purpose".
  Left alone; recorded so nobody "fixes" it later.

Nothing in the test surface should be unique and is not.

---

## 2. What the evidence actually points at

All five r45 failures share one shape: **a condition that never became true
within a wall-clock bound, with no loud error** — three `wait_track(0)`
expiries at a 5 s deadline, two 30 s silent stalls at `read_pcm`. That is the
flake class the M10 orchestrator note (2026-10-17) defined — "a
healthy-but-slow schedule tripping a wall-clock bound … A flaky test in the
suite is a defect of its own — it should wait on a condition rather than on a
wall-clock bound (or use a bound so generous that only a real stall reaches
it)" — and M24 proved it fires in this guest. M24/M25 fixed it in
`real_decode.rs` (LIVENESS 5 s → 30 s), the lib's `src/resident.rs`
(LIVENESS 5 s → 30 s), and `dsp_roundtrip.rs` (LIVENESS 5 s → 30 s). What the
fixes left behind, in exactly the binaries that failed in r45:

1. **`crates/rustamp-audio/tests/resident.rs` still constructed its
   in-process control peers with `Liveness::default()`** — interval 100 ms,
   **timeout 5 s** (`rustamp-mediahost/src/control.rs:32-39`) — at `:197` and
   `:207` (now fixed). M24's card explicitly deferred this file ("The
   `tests/resident.rs` harness … out of this card's three tests") and M25's
   hand-off list did not include it, so the last 5 s liveness trip wire in
   the resident test surface survived both waves. The failing test
   `resident_to_inprocess_source_change_is_gapless` (`:473`) runs behind
   exactly those peers. Mechanism (M25 §1.1's erasure path): a >5 s
   scheduling blackout trips the liveness → the client peer fails sticky →
   the decode thread's start is erased or the session dies → `wait_track(0)`
   (5 s deadline) can never succeed.
2. **The audio harnesses still waited on 5-second wall-clock bounds.** The
   lib's `dsp_hosted_tests` passed `WAIT = 5 s` as the adapter's per-block
   round-trip bound (`DspHarness::new(…, WAIT)` → `HostDspChain` block
   timeout) and used 5 s deadlines for its condition waits (`wait_track`,
   `wait_buffered`, `settle_seek`). M25 §2 named this residual and left it.
   A >5 s host-thread blackout during the decode thread's first block
   round-trip latches the `DspFailureWatch` before the engine's poll
   processes `TrackStarted`; `join_decode` then discards it, `duration_frames()`
   is never published, and `wait_track(0)` fails — the same erasure symptom
   with a second latch source. `hosted_tests`' 5 s read/seek timeouts are the
   literal shape of the original M10 flake ("unexpected error: media host wait
   timed out at a five second bound"). The integration file's `WAIT = 5 s`
   covered the same shapes (wait deadlines, `HostDecoder` read/seek timeout).
3. **Both audio harnesses still used pump-count proxies** (`wait_until(Stopped,
   N)`) for the run-finish waits — the shape M24 §1.1 proved flaky ("the
   number was an arithmetic guess about a proxy, not a derivation of what the
   run needs"). The failing test carries one at its next wait (`:497`), so
   fixing only `wait_track` would have moved the failure inside the same test.
4. **The two `real_decode` 30 s stalls are the same class at the escape
   itself.** With LIVENESS at 30 s, a silent `TimedOut` (not a
   `Disconnected`) means the host worker delivered no PCM for 30 s while the
   control channel stayed healthy. Static trace of every blocking call in
   that worker: the resident read (symphonia via `RegistryResident`;
   `rustamp-decoders` has no statics or locks) and the infinite ring write
   (the test client drains continuously); the ring/wake loops re-derive the
   condition from the counters after every wake (`rustamp-ipc/src/shm/ring.rs:553-579`
   and `:931-953`), so no lost-wakeup path exists; every control/ring failure
   path surfaces loudly (M25 §1.3's trace). No in-code silent-stall path
   remains. Two tests stalling in the same run is systemic, not a per-test
   race: the stalls are consistent with a **>30 s guest/process freeze under
   load** (checkpoint, AV scan, host contention on the 4-vCPU guest), and
   which tests are mid-transfer when a freeze hits is scheduling-dependent —
   exactly why the failures move between runs and tests. The `read_pcm`
   helper at `real_decode.rs:155` is the messenger, not the cause: it
   correctly reports "no PCM within the bound"; no honest test-side bound
   absorbs a >30 s freeze. Only the VM can prove that (§5).

Why each earlier fix "moved" the failure: every round widened or re-armed
one bound while the rest of the 5 s wall-clock surface stayed; a loaded guest
trips whichever bound the currently-running tests still carry, so the red
moves to the next carrier. This lane removes the class from the test surface
of the three failing binaries (and from the mediahost suites M24 §2 handed
to this lane) instead of widening one bound at a time.

---

## 3. The fix (test surface only; no assertion weakened)

The M10-sanctioned shape, already established by M24/M25: wait on the
condition, with a bound so generous that only a real stall reaches it; the
liveness trip wire moves from 5 s to 30 s because a dead resident never
answers a Ping and is caught by the control EOF regardless. Every deliberate
fast bound is untouched (the wedge tests' 20 ms block bound, the
bounded-read tests' 20 ms/10 ms read bounds, the missing-Pong test's 200 ms
policy), and the two assertions that referenced the old 5 s `WAIT` value keep
their original 5 s strength explicitly.

| File | Change |
|---|---|
| `crates/rustamp-audio/tests/resident.rs` | New `LIVENESS` const (100 ms / `WAIT`) replacing `Liveness::default()` at both `ControlPeer::start` sites in `FakeResidentFactory::open` (the file M24 deferred); `WAIT` 5 s → 30 s (doc comment added) covering `wait_track`/`wait_buffered`/`settle_seek`/inline deadlines, the `HostDecoder` read/seek timeout and the reap bound; `wait_until` (pump-count proxy) replaced by `wait_pumping` (condition + stall escape, M24's shape) at its three `Stopped` call sites. |
| `crates/rustamp-audio/src/resident.rs` `hosted_tests` | `WAIT` 5 s → 30 s (doc comment added): read/seek timeouts, reap/`finish` bounds, `WAIT * 3` refusal reap. The `elapsed < WAIT` assertion in `bounded_wait_never_hangs_and_retry_delivers_pcm` keeps its original 5 s explicitly (its 20 ms read bound is the claim). |
| `crates/rustamp-audio/src/resident.rs` `dsp_hosted_tests` | `WAIT` 5 s → 30 s (doc comment added): the engine tests' per-block round-trip bound (removes the mid-start latch source of §2.2), `wait_track`/`wait_buffered`/`settle_seek`/`wait_pumping` deadlines, reap/`finish` bounds; `LIVENESS.timeout` now `WAIT` (30 s, doc updated). `DspHarness::wait_until` (pump-count proxy) removed; its two engine-test `Stopped` call sites now use the existing `wait_pumping(…, WAIT)` (M24's helper, same condition). The wedge test's own 20 ms bound is unchanged. |
| `crates/rustamp-mediahost/tests/pcm_transfer.rs` | New `LIVENESS` const (100 ms / `STALL_WAIT`) replacing `Liveness::default()` at all five `ControlPeer::start` sites (M24 §2's hand-off to the mediahost lane). The 5 s `WAIT` const is gone; its uses (seek bound, three control-recv bounds, one reap bound) now use `STALL_WAIT` (30 s). The `elapsed < WAIT` assertion in `missing_pong_is_bounded_even_with_an_open_control_stream` keeps its original 5 s explicitly (its 200 ms liveness policy is the claim). |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | New `LIVENESS` const (100 ms / `STALL_WAIT`) replacing `Liveness::default()` at the module's two `ControlPeer::start` sites. |
| `crates/rustamp-mediahost/tests/pcm_process.rs` | New `LIVENESS` const replacing `Liveness::default()` at both `DecoderProcess::spawn` sites; the explicit 5 s seek bound now uses `STALL_WAIT`. |
| `crates/rustamp-mediahost/tests/dsp_process.rs` | New `LIVENESS` const replacing `Liveness::default()` at both `DspProcess::spawn` sites. |

Not changed, deliberately:

- `real_decode.rs` and `dsp_roundtrip.rs`: already fully on the 30 s shape
  (STALL_WAIT/JOIN_WAIT/SEEK_WAIT, LIVENESS 30 s) — no edit needed.
- Production liveness policies: `src/resident.rs:558` and `:1126` (the
  `DecoderProcess`/`DspProcess` spawn defaults), `src/bin/{dechost,dsphost}.rs`,
  `rustamp-dechost/src/main.rs`, `rustamp-dsphost/src/main.rs` — product
  policy, M25's precedent.
- The fixture examples' child-side policy (`examples/decoder_fixture.rs:16`,
  `examples/dsp_fixture.rs:17`, `Liveness::default()`): a product-policy-shaped
  choice inside a test fixture; the parent-side peers the tests construct are
  fixed. Named for the owner.
- Named follow-ups, same 5 s class, **other audio binaries, no failure
  evidence, not touched**: `crates/rustamp-audio/tests/common/mod.rs:461-532`
  (the shared `Harness` wait helpers, 5 s), `crates/rustamp-audio/tests/engine.rs:1865,2322,2360,2370,2384,2405`
  (5 s deadlines) and its `wait_until` pump-count proxies
  (`engine.rs:536-2168`), `crates/rustamp-audio/tests/flush_contract.rs:21`
  (`WAIT` 5 s), `crates/rustamp-audio/tests/render_rt.rs:79` (5 s deadline).
  If one of those binaries ever goes red the same way, this card's §3 shape
  applies verbatim.

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file outside the owned paths was modified.

---

## 4. Assertions kept (none weakened)

- Every test condition is unchanged: the same bit-exact comparisons, positions,
  event and error assertions, handshake pins, loud-refusal reason pins, sticky
  EOS checks, generation checks.
- Every deliberate fast bound is unchanged: the wedge tests' 20 ms block
  bound (`src/resident.rs` `dsp_wedge_is_a_bounded_wait_and_poisons_the_chain`,
  `engine_fails_the_track_when_the_resident_wedges`),
  `bounded_wait_never_hangs_and_retry_delivers_pcm`'s 20 ms read bound and its
  `elapsed < 5 s` promptness assertion (kept at 5 s explicitly),
  `bounded_read_timeout_is_not_eos_and_retry_delivers_pcm`'s 20 ms,
  `bounded_dsp_read_timeout_is_not_eos_and_retry_delivers_pcm`'s 10 ms,
  `missing_pong_is_bounded_even_with_an_open_control_stream`'s 200 ms liveness
  policy and its `elapsed < 5 s` assertion (kept at 5 s explicitly).
- The moved bounds are all stall escapes / liveness trip wires / reap bounds /
  pump-count proxies — none of them is a claim a test asserts. A dead
  resident/helper never answers a Ping and is caught by the control EOF
  regardless, so the 30 s liveness still detects death; only a
  healthy-but-slow schedule stops tripping it.
- The `wait_until` → `wait_pumping` conversions keep the identical condition
  (`transport() == Stopped`) and replace an arithmetic pump-count budget with
  the condition wait plus a generous wall-clock stall escape — M24's
  sanctioned shape, which M24 proved strictly better than the proxy.

---

## 5. What only the VM can prove, and how many runs

The suite runs in the Hyper-V test VM (4 vCPUs); this lane ran no test. The
orchestrator should run the full suite there (`cargo test --workspace
--all-features` from the checkout root in the guest) and check, per binary:

- `rustamp_audio` (lib): all `dsp_hosted_tests` engine tests
  (`engine_compensates_the_resident_latency_exactly`,
  `engine_gapless_seam_through_the_resident_chain_is_exact`,
  `engine_seek_through_the_resident_chain_lands_exactly`,
  `engine_fails_the_track_when_the_resident_dies`,
  `engine_fails_the_track_when_the_resident_wedges`) and all `hosted_tests`
  green, and no other `rustamp-audio` test regressed.
- `resident` (the `rustamp-audio` integration binary): all six tests green,
  especially `resident_to_inprocess_source_change_is_gapless`.
- `real_decode` (the `rustamp-dechost` integration binary): all seven tests
  green.
- `pcm_transfer`, `dsp_control`, `pcm_named`, `dsp_named` (the
  `rustamp-mediahost` test binaries) green, plus the ignored two-process
  suites via the runner's `--ignored` path (`pcm_process`, `dsp_process`,
  staged fixtures).
- `dsp_roundtrip` (the `rustamp-dsphost` integration binary) still green.

**A single green run is not evidence for a race.** The failure fired in 5 of
the last 5 runs while moving between tests, i.e. the 5 s class was
near-certain per run; after removing it, the residual risk is a >30 s guest
freeze (r45 showed two in one run). Recommendation: **at least 5 consecutive
full-suite green runs** with the three previously-red binaries green in every
one, plus 2–3 extra repeats of those three binaries under the standard
runner. If any run goes red, the failing line decides: a 5 s bound still
tripping means a missed site (the named follow-ups in §3); a 30 s stall
(`read_pcm`'s message) means a guest freeze — environmental, and the runner
should then log guest uptime/scheduler stalls (or the escape needs raising /
the guest needs more vCPUs), not another test edit.

---

## 6. Files and line counts

Counts include blank lines; all inside the owned paths. `git diff --stat`:

| File | Lines (after) | Change |
|---|---:|---|
| `crates/rustamp-audio/src/resident.rs` | 2878 (was 2887) | `hosted_tests` + `dsp_hosted_tests`: `WAIT` 5 s → 30 s with doc comments; `LIVENESS.timeout` tied to `WAIT`; `DspHarness::wait_until` removed, its two call sites use `wait_pumping(…, WAIT)`; the bounded-wait promptness assertion kept at explicit 5 s. Test modules only — no production code changed. |
| `crates/rustamp-audio/tests/resident.rs` | 599 (was 581) | `LIVENESS` const at both `ControlPeer::start` sites; `WAIT` 5 s → 30 s; `wait_until` → `wait_pumping` at three call sites; five inline 5 s deadlines → `WAIT`. |
| `crates/rustamp-mediahost/tests/pcm_transfer.rs` | 553 (was 541) | `LIVENESS` const at five `ControlPeer::start` sites; 5 s `WAIT` const removed, its uses now `STALL_WAIT`; one promptness assertion kept at explicit 5 s. |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 261 (was 251) | `LIVENESS` const at the module's two `ControlPeer::start` sites. |
| `crates/rustamp-mediahost/tests/pcm_process.rs` | 104 (was 97) | `LIVENESS` const at both `DecoderProcess::spawn` sites; the 5 s seek bound → `STALL_WAIT`. |
| `crates/rustamp-mediahost/tests/dsp_process.rs` | 164 (was 155) | `LIVENESS` const at both `DspProcess::spawn` sites. |
| `docs/tasks/M26-parallel-rings.md` | this card | new |

---

## 7. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0; one `cargo fmt --all` pass
  applied first to re-wrap two lines that the edits shortened).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited test target — nothing
  was executed).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned.

---

## 8. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the six
modified files above plus this card:

```
 M crates/rustamp-audio/src/resident.rs
 M crates/rustamp-audio/tests/resident.rs
 M crates/rustamp-mediahost/tests/dsp_control.rs
 M crates/rustamp-mediahost/tests/dsp_process.rs
 M crates/rustamp-mediahost/tests/pcm_process.rs
 M crates/rustamp-mediahost/tests/pcm_transfer.rs
?? docs/tasks/M26-parallel-rings.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written, and no file outside the owned
paths was modified.

---

## 9. Provenance

Implemented only from the task cards (M10, M20, M22, M24, M25), the RFCs they
cite, and the clean repository code (read-only outside the owned paths). No
Winamp source, SDK headers, disassembly or forbidden-tree contents were read.
No third-party code copied. License impact: none (no dependency change;
`rustamp-audio` and `rustamp-dechost`/`rustamp-dsphost` stay
GPL-3.0-or-later, `rustamp-mediahost` stays MIT OR Apache-2.0).
