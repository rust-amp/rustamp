# M14: Host-framework adversarial review (decoder + DSP residents)

Scope: `crates/rustamp-mediahost/**` (framework: `control.rs`, `pcm.rs`,
`decoder.rs`, `dsp.rs`, both `src/bin/*`, in-process tests), plus the two
resident crates `crates/rustamp-dechost/**` and `crates/rustamp-dsphost/**`.
Method: code inspection only plus the three allowed gates
(`cargo fmt --check`, `cargo clippy`, `cargo build`). **No `cargo test` was
run** (lane constraint); every test verdict below is a static assessment of
whether the test *could* fail, not an execution result. All two-process /
VM-only tests are `#[ignore]`d and were not executed here.

Gate receipts (this worktree): `cargo fmt --all -- --check` exit 0,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` exit 0,
`cargo build --workspace` exit 0.

## 1. Claim table

Verdicts: **CONFIRMED** = a written test fails if the claim breaks;
**PARTIAL** = pinned in one direction / one lane only; **PROSE-ONLY** = asserted
in M12/M13 but no test can fail on it; **INSPECTION** = holds by reading the
code, no test possible or present.

### Directed rings (the DSP lane's shared-framework change)

| # | Claim | Verdict | Evidence | Smallest reproduction / note |
|---|---|---|---|---|
| D1 | Directed rings carry both directions losslessly; wrong direction refused before any mapping opens | CONFIRMED | `crates/rustamp-mediahost/tests/dsp_named.rs:17` | Flip the expected direction in `open_directed` (`dsp_named.rs:28`) — the test fails. |
| D2 | Decoder defaults still offer `HostToMain` (decoder path byte-for-byte) | CONFIRMED | `crates/rustamp-mediahost/tests/dsp_named.rs:59`, `crates/rustamp-mediahost/src/pcm.rs:114,118` | Change `create`/`open` to another direction — `decoder_defaults_still_offer_host_to_main` fails. |
| D3 | Zero/invalid format, zero/`u32::MAX` capacity, corrupt size refused before allocating | CONFIRMED | `crates/rustamp-mediahost/tests/dsp_named.rs:72`, `crates/rustamp-mediahost/tests/pcm_named.rs:72` | Pass `capacity 0` — refused at `pcm.rs:154`. |
| D4 | Reused generation rejected on both paths | PARTIAL | `crates/rustamp-mediahost/src/decoder.rs:209`, `src/dsp.rs:274,285`, `src/dsp.rs:585,592`; open-level stale pinned at `tests/pcm_named.rs:43` | Open-level `Stale` is pinned. The *serve-level* consecutive-reuse guards (`previous_generation == Some(generation)`) have **no test that injects a duplicate generation** — both `TestRings` (`tests/pcm_transfer.rs:91`) and `NamedPcmRings` (`src/pcm.rs:153`) use fresh atomics, so the guard is dead code in every run. PROSE-ONLY for the serve-level check. |
| D5 | `TestRings` (decoder in-process double) enforces direction like production | REFUTED | `crates/rustamp-mediahost/tests/pcm_transfer.rs:121` vs `src/pcm.rs:218` | **F4.** `TestRings::open` checks only `frame_size`; a `MainToHost` offer to the decoder path returns `Ok` in-process but `Err(InvalidMessage)` on `NamedPcmRings`. See finding F4. |
| D6 | Bad spec (zero channels/rate) never reaches a ring on either path | CONFIRMED | `src/pcm.rs:75`, `src/decoder.rs:195`, `src/dsp.rs:148,400,570`; tests D3 | `validate_spec` runs before every `create*`; `open_impl` re-checks at `pcm.rs:203`. |

### Interrupt rename (`interrupt_seek` → `interrupt_reoffer`)

| # | Claim | Verdict | Evidence | Smallest reproduction / note |
|---|---|---|---|---|
| I1 | Decoder seek still wakes a worker blocked in infinite PCM I/O | CONFIRMED | `crates/rustamp-mediahost/src/control.rs:193`, `tests/pcm_transfer.rs:235` | `seek_interrupts_full_ring_and_discards_prefetch` uses a capacity-1 **full** ring + seek: without the monitor `close()` on `DecoderSeek`, the pump's `write_from_blocking(None)` never wakes and the test times out. |
| I2 | DSP reconfigure wakes a worker blocked in infinite PCM I/O | PARTIAL | `src/control.rs:193`, `src/dsp.rs:340`; `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:293` | The reconfigure test exists but runs **without contention** (500 frames read before reconfiguring, rings drained). Nothing fills both rings to full and then reconfigures, so the `ClosePair` wake path (`control.rs:193` + `pcm.rs:96`) is exercised only by inspection. A DSP analogue of the capacity-1 seek-contention test is missing. |
| I3 | `DspReset` deliberately never closes rings | CONFIRMED | `src/control.rs:191`, `src/dsp.rs:102`; `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:264` | `reset_keeps_rings_and_continues` asserts generations unchanged across reset — closing on reset would fail it. |
| I4 | `DspError` fails the monitor sticky (like `DecoderError`) | PROSE-ONLY (DSP) / CONFIRMED (decoder) | `src/control.rs:187`; decoder pinned by `tests/pcm_transfer.rs:260` | No DSP test feeds `DspError` inbound to a `ControlPeer` with a blocked reader. Decoder's `decoder_error_is_loud_and_never_eos` would hang without the `DecoderError` arm; the DSP arm has no counterpart. Mid-run DSP errors are inexpressible anyway (see F1). |
| I5 | No wake was lost by the rename; no healthy-path spurious close | INSPECTION | `src/control.rs:191`, all `ControlPeer::start` call sites pass `false` on player ends (`src/decoder.rs:624`, `src/dsp.rs:741`, `tests/pcm_transfer.rs:152`, `tests/dsp_roundtrip.rs:120`) and `true` on resident ends (`src/bin/dechost.rs:13`, `src/bin/dsphost.rs:14`, fixtures) | Decoder behaviour unchanged (Seek still closes); DSP gains the reconfigure wake. Cross-kind closes (`DspConfigure` seen by a decoder host and vice versa) only fire on protocol-violation paths that exit anyway — harmless over-close, noted in §2. |

### Liveness and abort

| # | Claim | Verdict | Evidence | Smallest reproduction / note |
|---|---|---|---|---|
| L1 | Player drop (graceful `Shutdown`) stops the host cleanly | CONFIRMED | `crates/rustamp-dechost/tests/real_decode.rs:369`, `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:330`, `tests/pcm_transfer.rs:169` | Drop without `Shutdown` handling would trip the 30 s `JOIN_WAIT` expiry. |
| L2 | Abrupt player loss (EOF, no `Shutdown`) stops the host without hanging | PARTIAL | DSP CONFIRMED in-process: `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:340`; decoder: PROSE-ONLY | The decoder has graceful-drop and fake-host-side loss tests but **no abrupt-player-vanishes-mid-pump test**; the DSP lane's test has no decoder mirror. Mechanism is shared (`ControlPeer`), but the blocked direction differs (decoder blocks in *write*, DSP in *read*), so sharing is not evidence. |
| L3 | Resident death mid-read is loud (`Disconnected`), never silent EOS | CONFIRMED in-process / UNPROVEN across processes | `tests/pcm_transfer.rs:281`, `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:406` | In-process fake-host tests pin the monitor-EOF → abort → `Closed` → `check()` chain. The real kill paths (`tests/pcm_process.rs:56`, `tests/dsp_process.rs:72`) are `#[ignore]` VM-only. |
| L4 | Explicit bounded waits are `TimedOut`, never false EOS; retry works | PARTIAL | Decoder CONFIRMED: `tests/pcm_transfer.rs:246`; DSP: PROSE-ONLY | No DSP test reads with `Some(timeout)`; `DspSession::read`'s `Ok(0) => TimedOut` arm (`src/dsp.rs:512`) is uncovered. |
| L5 | Messages arriving after abort fail loud; pending overflow fails loud | PROSE-ONLY | `src/control.rs:232`, `src/decoder.rs:402` | `set_abort`-after-failure and the 8-deep `pending` cap have no tests. |
| L6 | First failure sticks; EOF after an error keeps the error text | INSPECTION | `src/control.rs:58` (`get_or_insert`) | Holds by construction; no test sequences error-then-EOF. |

### Failure contract of both residents

| # | Claim | Verdict | Evidence | Smallest reproduction / note |
|---|---|---|---|---|
| F-C1 | Garbage/empty files are loud errors, never EOS; host ends `Err` | CONFIRMED | `crates/rustamp-dechost/tests/real_decode.rs:297`, `src/lib.rs:107` | `open` fails before any ring exists (`serve_inner` opens resident at `decoder.rs:193`, creates the ring at `:207`). |
| F-C2 | Unknown stage id / bad gain params refused before any ring; host ends `Err` | CONFIRMED | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:201,233`, `crates/rustamp-dsphost/src/lib.rs:117` | `open_chain` (`dsp.rs:143`) runs before the in-ring wait (`dsp.rs:249`). |
| F-C3 | Truncated file ends without hanging | PARTIAL (liveness pinned, loudness not) | `crates/rustamp-dechost/tests/real_decode.rs:326` | **F2.** The test explicitly accepts short-EOS **or** error (`terminal` is `"eos"` or `"error"`). A half-file can therefore present as clean EOS. |
| F-C4 | Seek past the end clamps and terminates | CONFIRMED | `crates/rustamp-dechost/tests/real_decode.rs:242`, `crates/rustamp-dechost/src/lib.rs:139` | Clamp-then-EOS is by design (inner decoder clamps); pinned. |
| F-C5 | Host exits rather than spins on every terminal path | CONFIRMED (in-process) | `serve_decoder`/`serve_dsp` return `Err`/`Ok` (no retry loops); every `JOIN_WAIT` unwrap in both suites | A spin would trip the named-stall expiry. Real-process exit codes unverified (VM). |
| F-C6 | `spec_wire`/`audio_spec` reject zero/oversize before any ring | CONFIRMED (unit) | `crates/rustamp-dechost/src/lib.rs:174`, `crates/rustamp-dsphost/src/lib.rs:224` | Pure unit tests; would fail on acceptance of `channels 0/300`. |
| F-C7 | Mid-run resident faults are loud with diagnostic text | PARTIAL | Decoder CONFIRMED (`fixture:fail` → `tests/pcm_transfer.rs:260`); DSP REFUTED-as-contract (see F1) | `ResidentChain::process` returns `()` (`dsp.rs:39`): a DSP stage **cannot** report a mid-run fault. Only panic/death (text lost, liveness still loud). |
| F-C8 | URL locations refused cleanly | PROSE-ONLY | `crates/rustamp-dechost/src/lib.rs:115` | No test feeds `MediaLocationWire::Url` to `RegistryResident::open`. One-line unit test missing. |
| F-C9 | No wire change; `PROTOCOL_VERSION` stays 3 | INSPECTION (+ Hello gate CONFIRMED at `tests/pcm_transfer.rs:382`) | `src/control.rs:158`, M12 §3 / M13 §3 | Absence-of-change holds by inspection (no `Message` edits in either lane); the old-Hello rejection test pins the version gate only. |

## 2. Findings, severity order

**F1 — Medium, PROVEN. DSP mid-run faults have no diagnostic channel
(`ResidentChain::process` is infallible).**
`crates/rustamp-mediahost/src/dsp.rs:39` declares
`fn process(&mut self, buf: &mut [f32]);` — no `Result`. The decoder boundary
is fallible (`ResidentDecoder::read -> Result`, `decoder.rs:43`) and every
failure becomes `DecoderError` text; the DSP boundary can only fail at `open`
(`dsp.rs:48`) or by panicking/dying (text lost, surfaced as generic
`Disconnected` via liveness). A stage that detects NaN/overflow/clipping
mid-run today must either stay silent or kill its own process to be heard.
This mirrors `rustamp-core`'s infallible `DspStage::process`, so fixing it
needs a core-contract RFC — but the M13 card never names the asymmetry, and
no test can cover a loud mid-run DSP error because none is expressible.
Smallest reproduction: try to write a `ResidentChain` whose `process` fails
after block *n* with `"clipping detected"` — there is no API to return it;
`serve_dsp`'s `DspError` send (`dsp.rs:220`) is reachable only from `open` /
control paths.

**F2 — Medium, PROVEN. A truncated file may present as clean short EOS.**
`crates/rustamp-dechost/tests/real_decode.rs:342` builds the terminal as
`loop { Ok(0) => "eos", Err => "error" }` and only asserts
`frames_total < 4000`. Symphonia returning `Ok(0)` on a cut file is
indistinguishable from clean EOS at the framework level, so a half-downloaded
track plays short with no diagnostic. Smallest reproduction is the test
itself: truncate the WAV helper at half length, serve via `serve_decoder` +
`RegistryResident`, read to `Ok(0)` — no error surfaces on the `"eos"` leg.

**F3 — Medium, PROVEN (impact latent). `DspReset` never wakes a worker blocked
in an empty-ring read, so reset latency is unbounded while idle.**
`crates/rustamp-mediahost/src/dsp.rs:176` blocks in
`read_into_blocking(&mut block, None)`; `try_recv` is polled only *before*
the read and between write chunks. `DspReset` deliberately never closes rings
(`control.rs:191`), so a reset sent while the host is idle-blocked sits
queued until the next player write. Today's stages have no-op `reset`, hence
unobservable — but the first stateful stage (EQ history, limiter look-ahead)
keeps stale state across a track boundary until data resumes. Smallest
reproduction: in-process `serve_dsp` with a stateful test chain counting
`reset()` calls, send `DspReset` while the in-ring is empty, observe the reset
not landing within any bound. The decoder has no analogue (seek *does* close).

**F4 — Low, PROVEN. The decoder in-process double is weaker than production:
`TestRings::open` ignores `RingDirection`.**
`crates/rustamp-mediahost/tests/pcm_transfer.rs:121` destructures only
`{ name, frame_size, .. }` and never checks `direction`, while production
`open_impl` refuses mismatches (`src/pcm.rs:218`). A regression offering a
`MainToHost` decoder ring would pass all eleven `pcm_transfer` tests and be
caught only by the Windows-only `pcm_named` suite. Smallest reproduction:
`TestRings::open` on a `RingOffer` with `direction: MainToHost` returns `Ok`;
`NamedPcmRings::open` on the same offer returns `Err(InvalidMessage)`. Fix is
a three-line direction assert mirroring `dsp_named.rs:28`.

**F5 — Low, PROVEN (gap). URL refusal and several symmetric edges are
prose-only.**
`RegistryResident::open` refuses `Url` (`crates/rustamp-dechost/src/lib.rs:115`)
but no test feeds it a `Url`; the decoder has no abrupt-player-loss test to
mirror `dsp_roundtrip.rs:340`; the DSP has no bounded-read `TimedOut` test to
mirror `pcm_transfer.rs:246` and no mid-run-error test to mirror
`pcm_transfer.rs:260` (blocked by F1). Each is a one-test addition; none
affects the wire.

**Over-close note (no finding).** The monitor closes on `DspConfigure` even on
a decoder host and on `DecoderSeek` even on a DSP host (`control.rs:193`).
Both only fire on protocol-violation paths where `command()` then returns
`InvalidMessage` and the host exits anyway — harmless, no healthy path
affected.

## 3. What the two lanes share that one lane could not see

- **Fallible decode vs infallible DSP (F1).** M12's boundary returns `Result`
  per block; M13's `process` returns nothing. The asymmetry lives in the
  shared `serve_*` error contract (`DecoderError` text always available,
  `DspError` text only pre-ring) and is invisible to either card alone.
- **One wake rule, two directions.** The decoder blocks in *write*, the DSP in
  *read*; both rely on the same monitor-close, but only the decoder has a
  full-ring contention test (I1 vs I2). The shared `ClosePair`
  (`pcm.rs:96`) is DSP-only tested under no contention.
- **Test-double drift (F4).** `TestRings` predates directed rings and was never
  taught the direction check the DSP lane added to production. Shared
  `PcmRings` defaults (`pcm.rs:41`) make this easy to miss: the default
  `create_directed`/`open_directed` cover `HostToMain` via the plain methods,
  so the decoder suite stays green while diverging from `NamedPcmRings`.

## 4. Cannot be verified without the VM or two real processes

- Spawning the real `rustamp-dechost-symphonia` / `rustamp-dsphost-stages`
  binaries through `DecoderProcess`/`DspProcess` (both `#[ignore]`,
  `tests/pcm_process.rs:22`, `tests/dsp_process.rs:35`), including abrupt
  resident kill mid-stream and real exit codes.
- Any run touching `RUSTAMP_TESTDATA` media or the `sine.flac` golden
  comparison behaviour under load (the M10 5 s flake note concerns the
  fixture path, not the real-decode path, which correctly uses `None` reads
  + 30 s join bounds).
- `xtask package` shipping the new binaries; missing-helper degradation;
  engine-side `Decoder`/`DspStage` adapters and latency compensation with a
  nonzero-latency chain (both resident stages report 0; RFC acceptance "reported
  latency matches measured delay" has no measuring test).
- Non-Windows (M6) wakes/mappings; spawn cost / one-process-per-decoder policy;
  hung-codec-with-live-`Pong` (documented limitation, M10 §"Failure and
  lifetime notes").
- Performance or stress behaviour: all capacities are 1–7 frames by design;
  no contention, soak, or multi-generation stress run was executed here.

## 5. Verdict counts

- Claims CONFIRMED by a test that can actually fail: **19**
  (D1, D2, D3, D6, I1, I3, L1, L3-in-process, F-C1, F-C2, F-C4, F-C5-in-process,
  F-C6, plus the nine resident round-trip/handshake/reset/drop tests counted
  individually in §1).
- Claims PARTIAL (pinned one lane / one direction only): **7**
  (D4, I2, L2, L4, F-C3, F-C7, F-C9-gate-half).
- Claims PROSE-ONLY (no failing test behind them): **5**
  (I4-DSP-half, L5, F-C8, D4-serve-level-half, L2-decoder-half).
- Findings: **5** (2 Medium contract-level in F1/F2, 1 Medium latent in F3,
  2 Low in F4/F5), all marked PROVEN above; no finding required execution to
  establish. Most serious: **F1** — the DSP boundary cannot report a mid-run
  fault with text, so the "every failure is loud" contract holds for the
  decoder but is structurally unavailable to the DSP resident.

## 6. Boundary receipt

Only this card was written; no code was changed and nothing was committed.
`git status --short` inside this worktree (proof elicited after writing):

```
?? docs/tasks/M14-host-framework-review.md
```

Gates: `cargo fmt --all -- --check` exit 0;
`cargo clippy --workspace --all-targets --all-features -- -D warnings` exit 0;
`cargo build --workspace` exit 0; `cargo test` not run.
