# W181 — The resident restart policy (RFC 0006 §8)

**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave181-audio-resident-restart`.

## Boundary and scope

Owned paths: `crates/rustamp-audio/**` and this card. **Nothing in
`rustamp-mediahost` was changed** — the control plane did not need a change,
see "Why no media-host change" below.

Nothing under `D:\code\Rustamp` was written; `D:\code\rustamp-research`, the
Winamp source, the SDK headers and any disassembly were never read. No
download, no window, no audio device, no git write, no commit.

Sources: RFC 0006 (§5, §8), RFC 0002, RFC 0003, RFC 0007 and the
repository's own code and task cards (M15, W175). No copied third-party code.

## The clause that was implemented

RFC 0006 §8, *Processes, licenses, crash behaviour*, verbatim:

> - Supervision: the engine spawns the helper, runs `Hello`/`Ping` liveness on
>   the control channel, and kills and reaps it on close. A dead helper looks
>   like a stalled peer at the ring layer (RFC 0003 §12: crash detection is
>   the control channel's job); the engine turns a missed `Pong` or an
>   unexpected close into `DecoderError`/`DspError` semantics and fails the
>   track, never the player. **Restart policy (retry once, then fail the
>   track) is engine policy, not contract.**

and from the same section's erratum bullet:

> Nothing else in this section changes: the helper is still spawned,
> supervised and killed by the engine, talks only over the documented
> protocol plus shared memory, and its death fails the track, never the
> player.

That sentence is the whole mandate: the RFC fixes *that* the engine restarts
the helper once and then fails the track, and it explicitly leaves the
policy itself to the engine. The one §5/§8 item W175 left open is closed
here.

## What was implemented

### `crates/rustamp-audio/src/resident.rs`

* **`RestartingDecoder`** — a `Decoder` that wraps the resident adapter
  (`HostDecoder` / `ProcessDecoder`) and owns the restart policy:
  * a mid-track failure of the resident is **retried once**: a fresh helper is
    spawned, the same source is reopened, and the track continues at the last
    position the engine had taken from it;
  * a **second** failure ends the track loudly with the existing `Decode`
    error (`"resident restart did not save the track: …"`). The budget is per
    track, so the retry never loops;
  * a `FormatChanged` is a transition, not a failure: it never restarts
    (RFC 0002 C3);
  * the wrapper forwards every other call to the decoder in use, so the engine
    keeps driving one `Decoder` and there is no second decode path
    (RFC 0006 §10.4).
* **`ResidentReopen`** — the seam the adapter uses to bring the helper back.
  The production implementation (`ProcessReopen`) spawns a fresh helper
  process for the same `MediaLocationWire`; the tests open a fresh in-process
  session. The adapter knows *that* a helper has to come back, not *how* one
  is spawned.
* **`ResidentConfig::restarts` / `with_restarts`** and
  **`DEFAULT_RESIDENT_RESTARTS`** (= 1) — the budget is engine policy, like
  `ResidentConfig::liveness` (W175's precedent). Zero switches the restart
  off.
* `ResidentFactory::open` now wraps the spawned helper in the
  `RestartingDecoder`, so the production path has the policy.

The DSP resident seam (§6) is untouched: RFC 0006 §6 is a separate contract
and W175's own choice #2 already left the DSP side alone ("that is §6, its
helper owns the other side of it"). The restart policy is about *tracks*, and
a track is a decoder.

### `crates/rustamp-audio/tests/resident.rs`

The fake resident now records what it did (one entry per opened decoder, with
the seeks it received), the test factory wraps its decoder in the
`RestartingDecoder` and implements `ResidentReopen` by starting a fresh host
thread for the same file.

### `crates/rustamp-audio/tests/resident_process.rs`

The VM-only two-process test now also pins that the restart happened at the
process level: the loud error of a helper that died mid-stream must name the
restart (`"resident restart …"`). The test stays `#[ignore]`d — it spawns a
process and belongs to the Hyper-V test VM.

## The ambiguities and the narrow readings

RFC 0006 §8 says "retry once" and nothing else. Four points were open; each
is resolved here as narrowly as the text allows.

1. **Where does the track resume?** The RFC does not say. The narrow reading
   is a retry of the *helper*, not of the *track*: the track continues at the
   last position the engine had taken from the resident (the number of frames
   the engine received since the last seek), so no audio is replayed and none
   is skipped. The wire quantizes the resume to whole milliseconds, so the
   resume lands within half a millisecond of that position — the same
   tolerance as any seek. A restart from the track's start would replay audio
   the listener already heard, which is why it was not chosen.
2. **What counts as a failure worth a retry?** The RFC names the death
   signals ("a missed `Pong` or an unexpected close"), but the media-host
   framework reports a dead helper, a wedged helper and a helper that
   reported a decode failure through one and the same channel (the sticky
   `Disconnected` of the control peer). The engine therefore cannot tell them
   apart without a change to the media-host contract, so the policy retries
   *any* mid-track failure of the resident once — which is exactly "try once
   more, then drop the track". A corrupt file therefore costs one extra
   helper spawn before the same loud error is reported.
3. **What is lost when a helper dies?** Nothing that was audible: the frames
   the dead helper had produced but not yet delivered are re-produced by the
   restarted helper, because the resume position is the last position the
   *engine* had taken (which is before them). The imprecision is the
   millisecond the wire quantizes the resume to — the same tolerance as any
   seek.
4. **A resident that cannot seek.** A restart would replay the track from its
   start, so a non-seekable resident is **not** restarted once it has
   delivered audio: the track ends instead. Before the first frame there is
   no position to resume at, so the restart happens without a seek.

Two more engine-side decisions, both documented in the module:

* The policy covers a failure *while the track plays*. A failure during a
  seek is the engine's seek path (it reports the failed seek and keeps
  playing); the next read then triggers the restart, which resumes at the
  position before the seek.
* A reopened helper that reports **another format** ends the track: the
  engine adopted the old format when it opened the track, and a new format
  would be decoded with the wrong converter.

## Why no media-host change

Distinguishing "the helper died" from "the helper reported a decode failure"
would need a new `MediaHostError` variant (the control peer's sticky failure
is a plain `String` today) — a change in the permissive framework crate, its
wire contract and its pinned tests, for a distinction the policy does not
need (see ambiguity 2). The lane therefore stayed in `rustamp-audio` and the
media-host contract is untouched.

## Tests

Headless (no process, no window, no audio device):

| Test | What it proves |
|---|---|
| `resident::restart_tests::a_failed_resident_is_restarted_once_at_the_last_known_position` | the retry happens and resumes at the last known position (exactly) |
| `resident::restart_tests::a_second_failure_fails_the_track_and_the_retry_never_loops` | the second failure ends the track loudly; the retry does not loop |
| `resident::restart_tests::a_healthy_resident_is_never_restarted` | a healthy track is never replaced and never seeked |
| `resident::restart_tests::a_resident_that_cannot_seek_is_not_restarted_after_it_delivered_audio` | ambiguity 4 |
| `resident::restart_tests::a_failure_before_the_first_frame_restarts_without_a_seek` | there is no position to resume at yet |
| `resident::restart_tests::a_format_change_is_a_transition_and_never_restarts` | a format change is not a failure |
| `resident::restart_tests::a_restart_budget_of_zero_switches_the_restart_off` | the budget is the engine's own |
| `resident::restart_tests::the_restart_budget_is_the_engine_s_own` | the default is the RFC's one |
| `tests/resident.rs::a_resident_that_dies_mid_track_is_restarted_once_and_then_fails_the_track` | the engine level proof over the real media-host stack: the helper is reopened **once**, the retry resumes at the last known position, the second death ends the track with a `Decode` error, and the player carries on with the next track |
| `tests/resident.rs::a_healthy_resident_track_is_never_restarted` | a normal track is unaffected: one resident, no seek, the same bit-exact audio |

The engine-level position is asserted as a *window*, not an exact value: the
engine may still hold audio in the ring that the dead helper had published
(up to the prefetch), so the resume is at most that window behind the death
point. The exact position is pinned by the adapter-level test above.

The two-process proof stays VM-only (`#[ignore]`d) and now also asserts that
the restart happened.

## Gates

* `cargo fmt --all -- --check`: exit 0.
* `cargo clippy -p rustamp-audio --all-targets --all-features -- -D warnings`:
  exit 0.
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0 (the workspace still compiles; no foreign crate needed a change).
* `cargo test -p rustamp-audio --all-features -- --test-threads=1`: green —
  94 unit + 3 + 55 + 5 + 13 + 1 + 7 + 1 + 8 + 9 (`resident`) + 1
  (`resident_process`, 5 ignored) + 17 + 1 doc test. Ten tests are new: the
  eight adapter-level policy tests and the two engine-level ones.
* `cargo test --workspace --all-features -- --test-threads=1`: green.
* The ignored two-process tests were **not** executed here: they spawn a
  process and belong to the VM by rule.

### Known flake (pre-existing, not from this lane)

`tests/resident.rs::resident_to_inprocess_source_change_is_gapless` can hit
its 30 s stall escape under heavy CPU load. Measured on this machine with
four busy loops: the **unmodified** `main` fails 6 of 25 runs, this branch 1
of 25 — the same flake, unchanged by this lane. It is the resident flake the
suite's `--test-threads=1` rule is about.
