# RFC 0014: Fallible DSP stages — a loud mid-run error channel

Status: **Proposed**
Owner: Integration Lead. Written by: task card
[`docs/tasks/M16-dsp-error-rfc.md`](../tasks/M16-dsp-error-rfc.md).
Depends on: RFC 0001 (core contracts), RFC 0002 (audio contracts),
RFC 0006 (out-of-process hosting), RFC 0012 (output error hook).
Closes: finding **F1** of [`docs/tasks/M14-host-framework-review.md`](../tasks/M14-host-framework-review.md).

Scope: `rustamp-core` (`DspStage`), `rustamp-mediahost`
(`ResidentChain`, pump, `serve_dsp`), the engine DSP call site
(`rustamp-audio`), the in-tree stage implementations (`rustamp-dsp`,
`rustamp-dsphost`, `rustamp-core::mock`), and the error-kind mapping
(`rustamp-proto`). Nothing here changes the wire or `PROTOCOL_VERSION`.

---

## 1. Why

M14 proved (F1) that a DSP stage cannot report a mid-run fault with
diagnostic text: `DspStage::process` returns `()`
(`crates/rustamp-core/src/audio.rs:401`), and the resident mirror
`ResidentChain::process` returns `()` too
(`crates/rustamp-mediahost/src/dsp.rs:39`). The decoder boundary is
fallible per block (`Decoder::read -> Result<usize>`, `audio.rs:153`) and
every failure becomes `DecoderError` text; the DSP boundary can only fail
at `open` (`dsp.rs:48`) or by panicking/dying, which loses the text and
surfaces only as a generic liveness `Disconnected`.

Consequences today:

- A stage that detects NaN/overflow/clipping mid-run must either stay
  silent or kill its own process to be heard (`dsp.rs:211-227` documents
  that native panics stay in the helper process; the text never crosses).
- Worse, silence is partially *masked*: the engine's `emit()`
  (`crates/rustamp-audio/src/decode.rs:635-659`) replaces non-finite
  samples with `0.0` (`decode.rs:647-654`), so a stage emitting NaN
  presents as clean quiet audio, not as a fault.
- `serve_dsp`'s `DspError` send (`dsp.rs:220`) is reachable only from
  `open` / control paths; no test can cover a loud mid-run DSP error
  because none is expressible (M14 §I4, §F-C7).

The hole mirrors the core `DspStage::process` contract, which returns
unit, so the decision is a core-contract one — hence this RFC, per
RFC 0001 §Decisions and `CONTRIBUTING.md` §Workflow.

Code evidence for the surrounding machinery this RFC reuses:

- `Error::Dsp(String)` already exists
  (`crates/rustamp-core/src/error.rs:30-32`) but is documented as
  configuration-time only ("rejected its configuration") and maps to
  `ErrorKind::Other` via the catch-all (`error.rs:71`).
- `serve_dsp` already converts any `serve_inner` failure into
  `Message::DspError { message }` with a bounded flush (`dsp.rs:218-227`),
  and the player side already surfaces it as a sticky loud failure
  (`handshake` doc, `dsp.rs:620-623`: "a resident refusal arrives as a
  sticky control failure carrying the `DspError` text").
- The engine already converts a decode-side `Error` into a player-visible
  event: `report_error` (`decode.rs:921-927`) sends `DecodeOut::Error`,
  and the decoder-error path ends the track loudly
  (`decode.rs:608-616`).

So both halves of the reporting path exist; only the *stage-to-caller*
channel is missing.

---

## 2. Options considered

### Option A: fallible `process` (recommended)

```rust
fn process(&mut self, buf: &mut [f32]) -> Result<()>;
```

(`Result` = `rustamp_core::error::Result` for `DspStage`,
`crate::Result` for `ResidentChain`.)

- **Engine:** `emit()` propagates the first stage error exactly like a
  decoder error: `report_error`, end the track (`Outcome::Ended`).
- **Resident:** `pump` propagates with `?`; `serve_dsp`'s existing
  `DspError` send carries the text to the player; the helper exits.
  The failure reuses the *open-time* loud path — no new message, no
  wire change.
- **Real-time cost:** none on the Ok path, which must stay
  allocation-free as today. `Err` allocates its message, which is
  acceptable because the run ends there — the same standing rule as
  `Decoder::read` errors, which already allocate `String`s on the
  decode thread. Unlike RFC 0012's `RenderFn` (a pure pull callback on
  the OS audio thread with no return channel), `process` runs on the
  decode thread (engine) or the host worker thread (resident), both of
  which can propagate `Result` with `?`.
- **Cost:** breaking. Every implementer and every ignoring call site
  must be touched (§6 lists them all; all are in-tree). This is also
  the cheapest moment to break it: no out-of-tree stage implementer
  exists yet, and classic `dsp_` bridging is explicitly future work
  (M13 §5), so waiting only strands more code later.
- **Precedent:** RFC 0002 C3 chose `read -> Result` over an `Event`
  for the same reason — the callee has no event sink, and `Result`
  already distinguishes "fine" from "failed".

### Option B: error latch plus polled accessor (RFC 0012 style)

```rust
fn stage_error(&self) -> Option<Error> { None }
```

The stage latches its failure; the engine checks after every block and
the resident pump checks after every `chain.process`.

- **Cost:** non-breaking (default `None`), but every caller grows a
  poll it can forget, and the error detaches from the failing call:
  ordering against already-processed blocks is ambiguous, and the
  contract must additionally specify post-failure output (bypass?
  silence? poisoned?) and latch-clearing (`reset`? `open`?) semantics.
- **Why not:** the latch exists in RFC 0012 because the audio thread
  *cannot* return values through `RenderFn`. That constraint does not
  apply here — `process` is an ordinary `&mut self` call on a thread
  that can propagate errors. Adopting the latch would be the complex,
  error-prone variation of Option A (the same degeneration RFC 0012
  §2 ascribes to its own Option B), with strictly more contract
  surface and strictly weaker failure attribution.

### Option C: keep `process` infallible (status quo, rejected)

Rule-level only: "stages must not fail mid-run; a fault is a bug, panic
if you must." Zero code change; F1 stays open permanently: panics lose
the diagnostic text by construction, and the NaN sanitizer keeps
masking silent corruption as quiet audio. Documents the hole instead of
closing it.

---

## 3. Recommendation and exact API

Adopt **Option A**.

### 3.1 `rustamp-core`: `DspStage::process` goes fallible

`crates/rustamp-core/src/audio.rs:401`:

```rust
/// Process interleaved frames in place. `buf.len()` is a multiple of the prepared channel count.
///
/// `Err` is terminal for the current run (see §4): the engine ends the
/// track loudly and the resident helper exits loudly. The Ok path must
/// not allocate; the Err path may, the run ends there.
fn process(&mut self, buf: &mut [f32]) -> Result<()>;
```

`Result` is already imported in `audio.rs` (`crate::error::{Error,
Result}`). `prepare`, `reset`, `name` and `latency_frames` are
untouched. The `Error::Dsp` doc comment (`error.rs:30-32`) is widened
from configuration-time to cover mid-run faults:

```rust
/// A DSP stage failed: refused configuration or a mid-run fault (RFC 0014).
#[error("dsp error: {0}")]
Dsp(String),
```

### 3.2 `rustamp-proto`: append `ErrorKind::Dsp`

`crates/rustamp-proto/src/error.rs:11-25`, appended at the end so
existing serialized indices do not move (the RFC 0002 C4 maneuver):

```rust
pub enum ErrorKind {
    Io,
    UnsupportedFormat,
    Decode,
    Output,
    Other,
    /// The stream changed format mid-stream (RFC 0002, C4). ...
    FormatChanged,
    /// A DSP stage failed mid-run or refused configuration (RFC 0014).
    /// Appended at the end so existing serialized values keep their index.
    Dsp,
}
```

And the classification (`crates/rustamp-core/src/error.rs:63-77`) maps
it explicitly instead of falling into `Other`:

```rust
Error::Dsp(_) => ErrorKind::Dsp,
```

`ErrorInfo` shape, `Event::Error` shape and every other variant are
untouched; a peer that never produces `Dsp` never emits the new index.

### 3.3 `rustamp-mediahost`: `ResidentChain::process` goes fallible

`crates/rustamp-mediahost/src/dsp.rs:35-42`:

```rust
pub trait ResidentChain: Send {
    fn latency_frames(&self) -> u32;
    /// Process interleaved frames in place; the frame count never changes.
    /// `Err` ends the helper loudly via the existing `DspError` path (§4).
    fn process(&mut self, buf: &mut [f32]) -> Result<()>;
    fn reset(&mut self);
}
```

`pump` (`dsp.rs:184`) becomes `chain.process(..)?`. Nothing else in
`serve_inner`/`serve_dsp` changes: the `Err` flows into the existing
`DspError { message }` send and bounded flush, and the helper exits
with `Err`. `DspResident::open`, the two-ring handshake, `DspSession`,
`DspProcess` and every `Message` are untouched.

### 3.4 `rustamp-dsphost`: adapt stage errors

`Chain::process` (`crates/rustamp-dsphost/src/lib.rs:153-169`) runs the
stages in order and maps the first failure, mirroring how `open` maps
`prepare` failures today (`lib.rs:195-197`):

```rust
fn process(&mut self, buf: &mut [f32]) -> Result<()> {
    for stage in &mut self.stages {
        stage
            .process(buf)
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
    }
    Ok(())
}
```

`Gain`/`Passthrough` keep their exact behavior and return `Ok(())`.

### 3.5 `rustamp-audio`: engine ends the track loudly

`emit()` (`crates/rustamp-audio/src/decode.rs:635-640`) treats a stage
error exactly like the decoder-error arm above it (`decode.rs:608-616`):

```rust
for stage in &mut self.dsp {
    if let Err(e) = stage.process(&mut self.in_buf[..samples]) {
        self.report_error(&e);
        if !self.flush_dsp() {
            return self.abandoned();
        }
        return Outcome::Ended;
    }
}
```

(`flush_dsp` reuses `emit`, so a stage that keeps failing during the
latency flush ends the track through the same arm; no second error is
published — first failure sticks, mirroring `control.rs:58` on the
resident side.) `set_dsp_chain` (`engine.rs:308`) keeps its signature.

---

## 4. The rule

1. **Terminal only.** `Err` from `process` ends the current run. A
   stage must not use it for per-block advisories, soft clipping
   warnings, or anything the run should survive. A non-terminal
   advisory channel, if ever needed, is a new RFC.
2. **First failure sticks.** The engine publishes one `Event::Error`
   per run; the resident sends one `DspError` per helper lifetime.
   Later failures on an already-failed run are dropped, never
   re-ordered ahead of the first.
3. **A failed chain/resident is poisoned.** After a mid-run chain
   error the helper has exited; the player must `reconfigure` (fresh
   chain, fresh rings) or drop the session. Resuming on the same
   chain is an error. This matches the open-failure behavior (M13
   §4): refusal before any ring already ends the helper.
4. **Panics stay bugs, never a reporting channel.** With a fallible
   method available, a stage that panics instead of returning `Err`
   is defective; liveness still fails loud (`Disconnected`) but the
   text is lost by the stage's own fault.
5. **The NaN sanitizer stays.** `decode.rs:647-654` remains defense
   in depth on the Ok path; it does not mask `Err`, which bypasses
   sample post-processing entirely.
6. **`reset`/`prepare` clear nothing about errors** because there is
   nothing to clear: errors are values returned to the caller, not
   state on the stage.

---

## 5. What does NOT change

- **The wire.** No `Message` added, removed or reshaped; no field,
  refusal reason or ring layout touched. Mid-run resident faults reuse
  the existing `DspError` message that open-time refusals already
  send. The resident speaks exactly the M13 §3 conversation.
- **`PROTOCOL_VERSION` stays 3** (M13 §3, M14 F-C9).
- **No new crates, no new dependencies, no licensing movement.**
  `rustamp-core` stays GPL-3.0-or-later; `rustamp-proto` stays
  MIT OR Apache-2.0 (the appended enum variant adds no dependency);
  `rustamp-mediahost` stays MIT OR Apache-2.0 (it links no decoder
  and no stage implementation — the `Chain` adapter lives in the
  GPL `rustamp-dsphost`, as M13 §"Now" requires).
- **No bypass, no auto-recovery, no UI policy.** The engine stops the
  run and publishes the error; what the main window shows and whether
  the user can retry is app-layer behavior on the existing
  `Event::Error` / `TransportState::Stopped` contracts (the same
  division RFC 0012 §4.3 draws).
- **No format semantics change.** `prepare`/`reset`/`latency_frames`,
  the two-ring handshake, generations, drain-then-sticky-EOS, and the
  `DspReset`-never-closes-rings rule are all untouched.

---

## 6. What breaks at each call site

All implementers and ignoring callers are in-tree (verified by
whole-tree search for `fn process(` and `.process(`). No out-of-tree
code can exist yet: classic `dsp_` bridging is future work (M13 §5),
and the `plughost` DSP symbols are host callbacks for classic plugins
(`host_dsp_is_active` / `host_dsp_process`,
`crates/rustamp-plughost/src/loader.rs:152-159,488-489`), not a
`DspStage` adapter — so **no plugin-bridge call site breaks**, but the
future bridge inherits §4: whatever a classic `dsp_` DLL does silently
today must surface as `Err(Error::Dsp(..))` or stay silent by explicit
rule, never by accident.

### Core contract (`rustamp-core`)

| Call site | Change |
|---|---|
| `src/audio.rs:401` | `fn process` gains `-> Result<()>` plus the doc rule |
| `src/error.rs:30-32` | `Error::Dsp` doc widened to mid-run faults |
| `src/error.rs:65-72` | `Error::Dsp(_) => ErrorKind::Dsp` replaces the `Other` fallthrough |
| `src/mock.rs:248` (`MockDsp::process`) | return `Ok(())` |
| `src/mock.rs:324` (test) | `dsp.process(..)` result must be handled |

### Engine chain (`rustamp-audio`)

| Call site | Change |
|---|---|
| `src/decode.rs:639` (`emit`) | handle `Result` per §3.5; the only production behavior change in this RFC |
| `src/engine.rs:308` (`set_dsp_chain`) | signature unchanged; doc note that stages run on the decode thread and `Err` ends the track |
| `tests/engine.rs:737` (`CountingDspStage`) | return `Ok(())` |
| `tests/engine.rs:1417` (`GainBoostDspStage`) | return `Ok(())` |
| `tests/engine.rs:1772` (`Poison`) | decide per test intent: keep panicking (still loud via liveness) or return `Err` to pin the new path |

### Resident chain (`rustamp-mediahost`, `rustamp-dsphost`)

| Call site | Change |
|---|---|
| `mediahost/src/dsp.rs:39` (trait) | `fn process` gains `-> Result<()>` |
| `mediahost/src/dsp.rs:184` (`pump`) | `chain.process(..)?` — the only framework behavior change |
| `dsphost/src/lib.rs:67` (`Gain`) | return `Ok(())` |
| `dsphost/src/lib.rs:95` (`Passthrough`) | return `Ok(())` |
| `dsphost/src/lib.rs:158` (`Chain`) | iterate with `?` per §3.4 |
| `dsphost/src/lib.rs:296,306,326` (tests) | handle the `Result` |

### Stage implementations (`rustamp-dsp`)

| Call site | Change |
|---|---|
| `dsp/src/eq.rs:429` (`Equalizer`) | return `Ok(())`; behavior identical |
| `dsp/src/eq.rs:545,564,574,582,589,592,605,649,675,702,721,730,731,746` (tests) | handle the `Result` (`unwrap` on success paths) |
| `dsp/src/limiter.rs:316` (`Limiter`) | return `Ok(())`; behavior identical |
| `dsp/src/limiter.rs:445,469,579,591,598,599,609,616` (tests) | handle the `Result` |
| `audio/tests/limiter_gain.rs:59` | handle the `Result` |

---

## 7. Acceptance criteria (for the implementing lane)

1. `DspStage::process` and `ResidentChain::process` return `Result<()>`;
   every in-tree implementer returns `Ok(())` with byte-identical
   behavior on success paths (existing EQ/limiter/gain/passthrough
   tests pass with only `Result`-handling edits).
2. **Engine:** a stage returning `Err(Error::Dsp("…"))` mid-run ends
   the track with `Event::Error` carrying `ErrorKind::Dsp` and the
   message text, transport leaves `Playing`, and no second error is
   published when the flush path re-fails. Pinned by a headless
   `rustamp-audio` test with a failing stub stage.
3. **Resident:** a chain failing after block *n* makes `serve_dsp`
   send `DspError` with the text and exit `Err`; the player-side
   `DspSession::read` fails loud (`Disconnected("resident DSP failed:
   …")`), never `Ok(0)`. Pinned by an in-process round-trip test
   mirroring `dsp_roundtrip.rs` (VM-gated two-process variant like
   `dsp_process.rs` if the lane needs it).
4. **No silent masking:** a stage emitting NaN *and* returning `Ok`
   still hits the `0.0` sanitizer (existing behavior, regression
   test); a stage returning `Err` bypasses sample processing
   entirely.
5. **Contract gates:** `PROTOCOL_VERSION` unchanged; no `Message`
   diff; `cargo fmt --check`, `cargo clippy --workspace --all-targets
   --all-features -- -D warnings` and the workspace test suite green
   (host suite in the VM per project rule; headless unit tests
   anywhere).
6. M14 finding F1 re-verified as closed: the M14 "smallest
   reproduction" (a `ResidentChain` failing after block *n* with
   `"clipping detected"`) compiles, and its text reaches the player.

---

## 8. Open questions for the owner

| ID | Question | What would settle it |
|---|---|---|
| **Q1** | **Stop vs bypass.** On a mid-run stage error the engine ends the track (§3.5, mirroring the decoder-error path). Should a failed stage instead be *bypassed* (rest of the chain continues on unprocessed audio) so playback survives a clipping-detector trip? | A product decision, not derivable from the code: is a DSP fault damage (stop, like a decode error) or telemetry (continue, like `skipped_packets`)? Bypass also needs a rule for what the failed stage outputs afterwards (silence? passthrough?) and would reopen the masking concern of §1. Recommendation stays **stop** until the owner says otherwise. |
| **Q2** | **`ErrorKind::Dsp` vs reuse `Other`.** §3.2 appends a `Dsp` kind so UI/plugins can distinguish DSP faults. If nothing downstream needs the distinction, the mapping can stay `Other` and `rustamp-proto` stays byte-identical. | Whether the main window, plugin authors' guide (P1/P2) or S10 IPC surface needs to tell "DSP failed" apart from "something failed". Check with the UI and plugin lanes; owner decides. Either way the message text already says `dsp error: …`. |
| **Q3** | **Resident restart policy.** After a mid-run chain error the helper has exited (§4.3); the player must reconfigure or drop the session. Should `DspSession` offer a convenience re-open (resend the last configure on the same session), or is an explicit fresh `open`/`reconfigure` by the caller enough? | Packaging/UX input: how expensive helper respawn is and whether the player auto-retries anything else today (it does not — cf. RFC 0012 Q1 fail-to-stop). Default: explicit caller action, no auto-retry. |
| **Q4** | **Are there out-of-tree stage implementers?** §6 claims all implementers are in-tree, which is what makes the breaking Option A cheap. Only the owner can confirm no partner/external code implements `DspStage` today. | Owner confirms. If external implementers exist, fall back to an additive `try_process` with a default body calling `process` (old impls compile; new failures override `try_process`) and accept the dual-method contract as the price. |

## 9. Provenance

Clean-room draft from in-tree sources only: RFCs 0001, 0002, 0006,
0012, 0013; cards M13 (resident) and M14 (finding F1); the cited
`rustamp-core` / `rustamp-mediahost` / `rustamp-dsphost` /
`rustamp-audio` / `rustamp-dsp` / `rustamp-plughost` code. No
dirty-room, reference-binary or third-party material was read or used.
No code was changed by this draft.
