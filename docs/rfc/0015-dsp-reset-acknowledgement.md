# RFC 0015: DSP reset acknowledgement — an ordering fence for `DspReset`

Status: **Proposed**. This is a wire change and is **not in force until the
project owner accepts it**; the implementation in the worktree
(`wave78-reset-fence-rfc`) is ready if he does, and nothing merges before
that decision.
Owner: mediahost lane (`rustamp-ipc` wire + `rustamp-mediahost` framework).
Written by: task card [`docs/tasks/M27-reset-fence.md`](../tasks/M27-reset-fence.md).
Depends on: RFC 0003 (shared-memory rings), RFC 0006 (out-of-process
hosting, §6.2 latency compensation and §8 crash behaviour), RFC 0007
(blocking consumer). Diagnosis: [`docs/tasks/A26-gapless-real.md`](../tasks/A26-gapless-real.md);
the fence was named earlier as the "true delivery fence" by
[`docs/tasks/A22-audio-rest.md`](../tasks/A22-audio-rest.md) §4.
Scope: `crates/rustamp-ipc` (one new `Message` variant, `PROTOCOL_VERSION`
3 → 4), `crates/rustamp-mediahost` (the pump's acknowledgement, the session's
bounded wait, the stub host's matcher), `crates/rustamp-plughost` (a
test-list entry only). **No change** to `rustamp-audio`, `rustamp-dsp`,
`rustamp-core`, the rings, the ring layout, or the decoder protocol.

---

## 1. Why

`DspSession::reset` sends a bare `Message::DspReset` and is documented as
*"not an acknowledgement or an ordering fence for writes on the independent
PCM channel"* (`crates/rustamp-mediahost/src/dsp.rs:573-576`, pre-change).
A26 traced what that costs at a gapless track start:

- The control path from `send` to pump-visibility crosses **three more
  threads** than the PCM path: client writer → byte stream → host reader →
  host monitor → the pump's `poll_commands`. The PCM path is one hop: decode
  thread → ring → the pump's `read_into_blocking` (A26 §1.3 step 4).
- At a gapless seam the track-end flush leaves the chain in its initial
  state, so a reset that lands **after** the new track's first block was
  processed clears the new track's **real** held frames and injects
  `latency` frames of silence outside the trim window. The trim bounds the
  stale *prefix*; it cannot bound the reset's *clearing* of real frames
  later in the track (A26 §1.3 steps 5-7, §1.4).
- The observed failure is `engine_gapless_seam_through_the_resident_chain_
  is_exact` (`crates/rustamp-audio/src/resident.rs:2743`): the seam is not
  bit-exact, first mismatch at slice index 924 — `latency` = 100 frames of
  silence at track 2's frames [924..1024]. The frame count survives, so the
  length assertions pass and only the value comparison fails (A26 §1.1-1.2).
- The seek path survives the same race only because its drain gives the
  control path milliseconds of slack; the seam has no drain between the
  flush and the first block (A26 §1.4).

| Item | Kind | Source |
|---|---|---|
| `DspSession::reset` is a bare `DspReset` send, documented as neither an acknowledgement nor an ordering fence | fact | `dsp.rs:573-580` (pre-change) |
| The pump applies `DspReset` at its control polls: top of loop, after reading a block and before processing it, during write-back — and in the post-EOS idle loop | fact | `dsp.rs:176`, `:192`, `:198`, `:368` |
| A late reset at a gapless seam clears real held frames and injects `latency` silence outside the trim window; the frame count survives | fact | A26 §1.3 steps 5-7 |
| The seek path's drain gives the control path slack; the seam has none | fact | A26 §1.4 |
| New `Message` variants need a `PROTOCOL_VERSION` bump; a lower-version peer would mis-decode them | fact | RFC 0003 §9, RFC 0005 §12, RFC 0006 §7 |
| Postcard encodes an enum by **declaration index** | fact | `rustamp-ipc/tests/hosting_wire.rs` file docs |
| `DspStage::prepare` leaves fresh state (the in-process stages do; RFC 0006 §6.2's trim/flush compensation assumes it) | fact | M19 §1, choice #6; A26 §3 |

**Why not the alternatives** (A26 §2, restated because they are the honest
competitors):

- **A sleep is forbidden** (owner rule) and only narrows the race — the
  exact mistake A20 made and A22 removed.
- **Skipping the reset at the seam** diverges from the in-process chain and
  re-opens the stateful-plugin seam click the reset exists to prevent.
- **A same-format reconfigure** would be synchronous but churns both rings
  per track start; the pinned contract of
  `dsp_prepare_with_the_same_format_clears_state_without_reconfigure`
  ("a same-format prepare never touches the rings") forbids it.
- **A probe cannot detect the landing** at the seam: the flush has already
  left the chain in its initial state, so the reset's output is identical
  whether it has landed or not (A26 §2).

No honest fix exists inside `rustamp-audio`; the fence belongs to the
framework that owns the wire.

## 2. What changes (this is a wire change)

1. **`Message::DspResetAck`** — a new **unit** variant, host → main,
   placed immediately after `DspReset` (`crates/rustamp-ipc/src/message.rs`).
   Request/answer adjacency has precedent: `Ping`/`Pong`,
   `DecoderSeek`/`DecoderSeeked`. Because postcard encodes an enum by
   declaration index, `DspError` moves from variant index 26 to 27.
2. **`PROTOCOL_VERSION` becomes 4** (from 3). The bump is mandatory, not
   bookkeeping: a v3 peer would mis-decode `DspResetAck` — index 26 is
   `DspError` in its enum, a struct variant, so it would mis-parse the
   frame — and would read a real `DspError` at the wrong index. The bump
   turns a mixed-version pair into a clean `Hello` rejection, the same
   reasoning as the 0 → 1, 2 → 3 bumps (RFC 0003 §9, RFC 0005 §12).
3. **The pump sends the acknowledgement after it applies the reset**, at
   every place it can be applied: in `poll_commands` (which covers all
   three pump poll sites) and in the post-EOS idle loop
   (`crates/rustamp-mediahost/src/dsp.rs`).
4. **`DspSession::reset` waits, bounded, for the acknowledgement** and
   consumes it (`crates/rustamp-mediahost/src/dsp.rs`). The signature is
   unchanged, so `rustamp-audio` compiles and behaves without edits.

**What a peer that predates this RFC sees.** At `Hello`, a v4 player and a
v3 helper reject each other: the monitor fails with "incompatible or
duplicate Hello" and the conversation fails sticky **before any `DspReset`
is sent** (`crates/rustamp-mediahost/src/control.rs:158-166`; the app side
enforces the same barrier in `rustamp-app/src/supervise.rs`). No mis-decode
is reachable: the pair never enters the DSP conversation. Within a v4
pair, `DspResetAck` is the only new frame on the control channel.

## 3. The rule

- `DspReset` is unchanged: main → host, rings and generations untouched,
  one per reset. `DspResetAck` is host → main, **exactly one per received
  `DspReset`, sent only after the chain state is cleared**. It carries no
  payload: the session is single-threaded (`&mut self`), so at most one
  reset is outstanding and no correlation token is needed.
- `DspSession::reset` returns `Ok(())` **only after the acknowledgement
  arrived**. The reset is then an ordering fence for the PCM channel: when
  it returns, no block written afterwards can be processed with pre-reset
  state, and no pre-reset state survives. This is the `DspStage` contract
  the adapter must deliver (`prepare` leaves fresh state, like the
  in-process stages).
- The wait **consumes** the acknowledgement: it never sits in the queue to
  trip the "unexpected DSP message while writing/reading" rejections
  (`dsp.rs:497-500`, `:536-539`, pre-change line numbers).
- **When the acknowledgement never comes**, the wait is bounded by
  `CONTROL_WAIT` (5 s — the same bound the handshake exchanges use). On
  expiry `reset` returns `Err(MediaHostError::TimedOut)` — a loud failure,
  **never a silent success** — and fails the conversation sticky
  (`ControlPeer::fail`), because the chain's state is then unknown (the
  reset may still land late). Every later call on the session returns
  `Disconnected` naming the reset acknowledgement; a late `DspResetAck`
  dies with the failed conversation and **can never be mistaken for an
  unexpected message** on a write or read. Production consequence: the
  adapter latches the failure, poisons the chain, and the engine fails the
  track — never the player (RFC 0006 §8).
- If the wait receives anything other than the acknowledgement, the
  conversation is desynced: `InvalidMessage`, and the peer fails sticky for
  the same reason.
- The host never acts on `DspResetAck`: it is a reply, not a command. The
  pump's `command()` matcher and the stub `DspHost` classify it explicitly
  as a protocol violation if it ever arrives host-side, so no matcher can
  mistake it for an unexpected message in the legitimate direction either.

## 4. What does not change

- `DspReset` itself: same meaning, same direction, rings untouched. The
  reset still lands between blocks and during the 20 ms control-poll
  slices, including while idle and after a drained EOS.
- The rings, the ring layout, `RING_VERSION`, generations, the decoder
  protocol, the plughost/classic-plugin path, `WinampIpc`.
- **`rustamp-audio`: no code, test, assertion, wait or bound changes.**
  `ChainControl::reset` calls `session.reset()`, whose signature is
  unchanged; the fence makes the engine seam test pass **as written**
  (A26 §2 steps 4-5). The audio lane's doc comments at
  `crates/rustamp-audio/src/resident.rs:823-834` and `:2249-2254` describe
  the pre-fence semantics and become stale once this lands; rewording them
  is the audio lane's call, not this one's.
- The trim/flush latency compensation (RFC 0006 §6.2) is unchanged; the
  fence removes the race it could not bound.
- The permissive-crate invariant: `rustamp-ipc`, `rustamp-mediahost` and
  `rustamp-plughost` stay MIT OR Apache-2.0 with no new dependency
  (RFC 0004; the variant is a unit — no new wire type, no new crate).

## 5. Acceptance criteria

1. **Wire:** `Message::DspResetAck` exists beside `DspReset` with a
   round-trip test at its declaration index (0x1a); `DspError` round-trips
   at 0x1b; the variant-count and sequential-index tests name 28 variants;
   `PROTOCOL_VERSION == 4` with the version history updated; the existing
   `Hello`-rejection gate now rejects a v3 peer (and a v4 peer rejects a
   v3 helper).
2. **The fence:** a reset with no PCM (idle), a mid-stream reset, and a
   reset after a drained EOS each return only after the chain applied them
   — the chain's own report is already observable when `reset` returns, and
   the wait is still blocked while the host's callback is held. Two resets
   queued behind a held callback are each acknowledged, in order, exactly
   one ack each. Rings and generations are untouched throughout.
3. **The timeout:** a resident whose reset never returns makes `reset`
   return `Err(MediaHostError::TimedOut)` within the bound — never a silent
   pass. The failure is sticky: later calls return `Disconnected` naming
   the reset acknowledgement, and the late acknowledgement never surfaces
   as an unexpected message. The host ends loud on the player's hang-up.
4. **Every matcher classifies the ack:** the stub `DspHost` (explicit
   arm), the pump's `command()` (explicit arm), `rustamp-plughost`'s
   foreign-message list (ignored, sends nothing). The decoder host's
   matchers need no change: a DSP ack arriving at a decoder host *is*
   unexpected and stays a loud `InvalidMessage`.
5. **No regression:** the `rustamp-mediahost`, `rustamp-dsphost` and
   `rustamp-audio` DSP suites stay green in the VM — in particular
   `engine_gapless_seam_through_the_resident_chain_is_exact` (the A26 red),
   `reset_keeps_rings_and_continues`, and the adapter-level probe tests
   (`dsp_delay_chain_round_trip_...`, `dsp_prepare_with_the_same_format_...`),
   which keep passing unchanged because the fenced reset makes their first
   probe succeed. Per M26 §5, the bar is **at least 5 consecutive green
   full-suite runs** in the guest, because the race is probabilistic.
6. **Gates:** `cargo fmt --all -- --check`, `cargo clippy --workspace
   --all-targets --all-features -- -D warnings`, `cargo build --workspace`
   green on the host; `cargo test --workspace --all-features` runs only in
   the Hyper-V VM, never on the host.

## 6. Open questions

| ID | Question | What would settle it |
|---|---|---|
| O1 | Is `CONTROL_WAIT` (5 s, the handshake bound) the right bound for the reset wait, or should the reset carry its own bound (e.g. the adapter's per-block timeout)? | A VM measurement of worst-case control delivery under load, in the M26 sweep pattern. Today the handshake uses the same 5 s and the pump polls commands every 20 ms, so the bound carries ~250 poll slices of margin. |
| O2 | On a timed-out reset, fail the conversation sticky (this RFC) or leave the session usable? | The owner's call. The alternative requires `write`/`read` to tolerate a stray `DspResetAck`, which this RFC rejects as silent; the sticky failure matches `reconfigure`'s existing precedent (fail the peer on error) and RFC 0006 §8's fail-the-track rule. |
| O3 | Insert `DspResetAck` beside `DspReset` (this RFC; `DspError`'s index moves 26 → 27) or append it at the end of the enum (index 27; `DspError` stays 26)? | The owner's preference. Both are safe under the `Hello` gate — a v3 peer never reaches the conversation — and adjacency keeps the request/answer pair readable, like `Ping`/`Pong` and `DecoderSeek`/`DecoderSeeked`. The hand-off (A26 §2 step 1) says "next to `DspReset`"; this RFC follows it. |
| O4 | Does the decoder protocol ever need the same fence (e.g. for a future flush command)? | Only if a decoder-side ordering need appears; today seek and format change use ring generations, which are already a fence. No action now. |
| O5 | Should the acknowledgement carry a token, for future multi-reset pipelining? | Only if the session ever allows more than one outstanding reset; it is `&mut self` and single-threaded by contract, so a unit variant suffices. |

## 7. Provenance

Designed from our own material only: the task cards A22, A26, M19 (and the
RFCs they cite — RFC 0003 §§8-9, RFC 0005 §12, RFC 0006 §§4, 6.2, 7, 8,
RFC 0007), the RFCs 0006/0007/0012 read for shape, and the clean repository
code (`crates/rustamp-ipc/src/message.rs`, `crates/rustamp-mediahost/src/
{dsp,control,lib}.rs`, `crates/rustamp-plughost/src/control.rs`, the test
suites). No Winamp source, SDK material, documentation or disassembly was
read; no reference binary was run; nothing was downloaded. No third-party
code was copied; no dependency was added. License impact: none — the five
permissive crates stay MIT OR Apache-2.0, and the touched crates stay
within their current licenses.
