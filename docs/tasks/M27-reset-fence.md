# M27 — The DSP reset fence: RFC 0015 drafted, implemented, waiting for the owner

**Date** 2026-10-18. **Lane** `wave78-reset-fence-rfc`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave78-reset-fence-rfc`.
Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status: implemented; the merge waits for the owner.** RFC 0015
(`docs/rfc/0015-dsp-reset-acknowledgement.md`) is a **proposal** — the wire
change (a new `Message` variant, `PROTOCOL_VERSION` 3 → 4) is **not in force
until the project owner accepts it**. The code in this worktree is the
implementation that is ready if he does; nothing here is committed or merged,
and no gate was run that the owner's acceptance would invalidate. fmt, clippy
and build gates passing. VM test execution pending the orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary. No test process, example,
player, helper binary or reference binary was spawned; no VM was touched; no
window or audio device was opened; no download, no dependency addition, no
install, no commit and no other git write. Only `cargo fmt --all` (one fix),
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings` and `cargo build --workspace --offline` ran
(receipts in §10). Every command ran inside this worktree.

---

## 0. Boundary & ownership

Owned paths: `docs/rfc/0015-dsp-reset-acknowledgement.md`,
`crates/rustamp-ipc/**`, `crates/rustamp-mediahost/**`,
`crates/rustamp-plughost/**`, and this card (`docs/tasks/M27-reset-fence.md`).

Read only (not modified): `crates/rustamp-audio/src/{resident.rs,decode.rs,
engine.rs}`, `crates/rustamp-mediahost/src/{control.rs,pcm.rs,decoder.rs}`,
`crates/rustamp-mediahost/tests/{pcm_transfer.rs,support/*}`,
`crates/rustamp-dsphost/tests/dsp_roundtrip.rs`, `crates/rustamp-dsphost/src/
{main.rs,lib.rs}`, `crates/rustamp-app/src/{host.rs,supervise.rs}`,
`crates/rustamp-plughost/src/{state_bridge.rs,host_window.rs}`,
`crates/rustamp-ipc/src/{transport.rs,lib.rs}`, and the cards A22, A26 (read
from the sibling worktree `wave77-gapless-real`, where that lane wrote it —
it is not in this worktree's tree), M19, M26, plus the RFCs 0003, 0005, 0006,
0007, 0012, 0014.

No file under `crates/rustamp-audio`, `crates/rustamp-dsp`, `crates/rustamp-core`,
`crates/rustamp-dsphost`, `crates/rustamp-dechost`, `crates/rustamp-app` or any
other crate was modified. No `Cargo.toml`, `Cargo.lock` or `THIRD_PARTY.md`
change (no new dependency of any kind).

Sources: the A26 card (the diagnosis and the hand-off, `docs/tasks/A26-gapless-
real.md`), the A22 card §4 ("true delivery fence"), the M19 card (the adapter
and its tests), RFC 0006 §6.2/§8, and the clean repository code only. No Winamp
source, SDK material, disassembly, research tree or reference binary was read;
no third-party code was copied; no dependency was added.

---

## 1. What was asked

A26 §2 handed the fix to this lane with exact steps: a `Message::DspResetAck`
unit variant next to `DspReset` in `rustamp-ipc` (a wire change needing an RFC
first), the pump sending the acknowledgement after it applies the reset at
every place it can be applied, `DspSession::reset` waiting for it with a bound
(a timeout is a loud `MediaHostError::TimedOut`, never a silent pass), the ack
consumed by that wait and never left in the queue to trip the "unexpected DSP
message while writing/reading" rejections, and every existing matcher updated
so the acknowledgement can never be mistaken for an unexpected message. Tests
for the fence itself and for the timeout. `rustamp-audio` needs no code change
(A26 §2 steps 4-5).

## 2. The RFC's decision and what it leaves open

`docs/rfc/0015-dsp-reset-acknowledgement.md` (**Proposed**) decides:

- **The wire change:** `Message::DspResetAck`, a unit variant, host → main,
  placed immediately after `DspReset` (request/answer adjacency, like
  `Ping`/`Pong` and `DecoderSeek`/`DecoderSeeked`). `DspError` therefore moves
  from variant index 26 to 27.
- **`PROTOCOL_VERSION` becomes 4** (from 3). The bump is mandatory, not
  bookkeeping: postcard encodes an enum by declaration index, so a v3 peer
  would mis-decode `DspResetAck` (index 26 is `DspError` in its enum) and read
  a real `DspError` at the wrong index. A pre-RFC peer sees a clean `Hello`
  rejection ("incompatible or duplicate Hello") and the conversation fails
  sticky before any `DspReset` is sent — no mis-decode is reachable.
- **The rule:** one ack per received `DspReset`, sent only after the chain
  state is cleared; `DspSession::reset` returns only after the acknowledgement
  arrived (the ordering fence for the PCM channel); the wait consumes the ack;
  the host never acts on it (explicit protocol-violation arms host-side).
- **What does not change:** `DspReset` itself, the rings/layout/generations,
  the decoder protocol, the plughost/classic path, the trim/flush compensation,
  and **`rustamp-audio` (no code, test, assertion or bound)** — the engine seam
  test passes as written once the fence lands.

It leaves open (each with what would settle it): **O1** the reset-wait bound
(`CONTROL_WAIT` = 5 s, the handshake bound — settled by a VM worst-case control
delivery measurement); **O2** sticky-fail vs usable session after a timed-out
reset (the owner's call — this RFC proposes sticky); **O3** insert-beside vs
append-at-end for the variant (the owner's preference — the hand-off said
"next to `DspReset`", this RFC follows it); **O4** whether the decoder protocol
ever needs the same fence (no need today — seek/format use ring generations);
**O5** whether the ack ever needs a token (no — the session is `&mut self`, one
outstanding reset).

## 3. What was implemented, and the protocol version

**`PROTOCOL_VERSION = 4`** (`crates/rustamp-ipc/src/message.rs:29`), with the
version history extended. The new variant sits at index 26 (0x1a);
`DspError` moved to 27 (0x1b).

1. `crates/rustamp-ipc/src/message.rs` — `Message::DspResetAck` beside
   `DspReset`; `PROTOCOL_VERSION` 3 → 4 with the history entry.
2. `crates/rustamp-mediahost/src/dsp.rs` —
   - `poll_commands` sends `Message::DspResetAck` after `chain.reset()`
     (one site covers all three pump poll sites: top of loop, before a block
     is processed, during write-back);
   - the post-EOS idle loop sends it after `chain.reset()` too;
   - `command()` gained an explicit `DspResetAck` arm (a reply, not a
     command: a DSP host never receives its own acknowledgement);
   - `DspSession::reset` now sends `DspReset` and waits bounded
     (`CONTROL_WAIT`) for the ack, consuming it; the doc rewritten from "not an
     acknowledgement or an ordering fence" to the fenced semantics;
   - the module doc gained the fence paragraph.
3. `crates/rustamp-mediahost/src/lib.rs` — the stub `DspHost` gained an
   explicit `DspResetAck` arm (per the hand-off: the catch-all stays, the arm
   is for clarity).
4. `crates/rustamp-plughost/src/control.rs` — `DspResetAck` added to the
   foreign-message test array (ignored, sends nothing) and named in the
   module doc's ignore list.
5. Wire tests: `hosting_wire.rs` (version 4, ack round-trip at 0x1a,
   `DspError` at 0x1b, 28 variants, sequential indices 15..=27) and
   `command_wire.rs` (version 4).

## 4. How a missing acknowledgement behaves

`DspSession::reset` waits `CONTROL_WAIT` (5 s — the same bound the handshake
exchanges use). If the acknowledgement never comes, `reset` returns
`Err(MediaHostError::TimedOut)` — **a loud failure, never a silent success** —
and **fails the conversation sticky** (`ControlPeer::fail`), because the
chain's state is then unknown (the reset may still land late). Every later
call on the session returns `Disconnected` naming the reset acknowledgement;
a late `DspResetAck` dies with the failed conversation and **can never be
mistaken for an unexpected message** on a write or read. If the wait receives
anything other than the ack, the conversation is desynced: `InvalidMessage`,
and the peer fails sticky for the same reason. Production consequence (RFC
0006 §8): the adapter latches the failure, poisons the chain, and the engine
fails the track — never the player.

## 5. Tests (written, not executed here)

All in `crates/rustamp-mediahost/tests/dsp_control.rs` (headless, one process,
the production `serve_dsp` pump against a stateful fake resident — the same
shape as the existing suites):

| Test | What it proves |
|---|---|
| `reset_returns_only_after_the_host_applied_it` | **The fence itself:** with no PCM and no hold, when `reset` returns the chain has already applied the reset (its own report is there via `try_recv`, no re-send, no wait); generations untouched; the consumed ack leaves the stream undisturbed (a block round-trips bit-exact). |
| `idle_reset_lands_without_pcm_and_clears_history_before_the_next_block` (restructured) | The idle/mid-stream/post-EOS application sites: the first reset is serviced with no PCM while its callback is held — the fenced wait is still blocked (`!handle.is_finished()`) and returns only after release; each later reset is applied when it returns (`try_recv` counts 2, 3, 4); cleared history processes the next blocks; generations unchanged; sticky EOS after the drained close. |
| `queued_resets_are_each_acknowledged_in_order_while_the_host_is_busy` | **The host-side half, over a raw transport:** two resets queued behind a held callback are drained in order and each answered with exactly one `DspResetAck` (the old test's "queued while held" sub-case, which the session can no longer produce — see §7). |
| `reset_without_an_acknowledgement_is_a_loud_bounded_timeout` | **The timeout:** a wedged reset callback makes `reset` return `Err(TimedOut)` within the bound; the host did enter the reset (count 1 already observable); the failure is sticky (`Disconnected` naming "reset acknowledgement" on later write/read); after release the late ack never surfaces as an unexpected message and the host ends loud on the player's hang-up. |
| `bounded_dsp_read_timeout_is_not_eos_and_retry_delivers_pcm` | Unchanged. |

Plus the wire tests in `crates/rustamp-ipc/tests/{hosting_wire,command_wire}.rs`
(ack round-trip at index 0x1a, `DspError` at 0x1b, 28 variants, version 4).

## 6. The exact places the acknowledgement had to be handled

Every `Message` matcher in the workspace was surveyed (the clippy
`--all-targets` gate compiles them all, so a missed exhaustive match would
not have compiled):

- **Sent:** `poll_commands` (`dsp.rs`, covers the pump's three poll sites) and
  the post-EOS idle loop (`dsp.rs:392-395`) — after `chain.reset()`, one ack
  per reset.
- **Consumed:** `DspSession::reset`'s bounded wait (`dsp.rs:601-622`).
- **Classified as a violation host-side:** `command()` (`dsp.rs:117-120`,
  explicit arm) and the stub `DspHost` (`lib.rs:158-161`, explicit arm).
- **Ignored where it can never legitimately arrive:** `rustamp-plughost`
  `handle_message`/`handle_message_inbound` (catch-alls already ignore it;
  pinned by the foreign-message test array) and `state_bridge::apply_message`
  (catch-all, untouched crate-side code) — the plugin host never runs a DSP
  session.
- **No change needed:** the decoder host's matchers (`decoder.rs` — a DSP ack
  arriving at a decoder host *is* unexpected and stays a loud `InvalidMessage`),
  `rustamp-app`'s `on_inbound` and `supervise.rs` (catch-alls / relative
  version comparisons), `message.rs::ring_expect` (`_ => None`), the
  `ControlPeer` monitor (forwards it to the incoming queue like any other
  non-liveness message).

## 7. Corrections to the hand-off (not followed blindly)

1. **The timeout step was incomplete.** A26 §2 step 3 said "a timeout is a
   loud `TimedOut, never a silent pass" but did not say what happens to the
   session and the *late* ack afterwards. Left as fire-and-forget-plus-
   timeout, a late ack would trip the "unexpected DSP message while
   writing/reading" rejections — exactly what the task forbids. This lane
   makes the failure **sticky** (`peer.fail`, matching `reconfigure`'s
   precedent), so the late ack can never surface as an unexpected message;
   RFC 0015 §3 states it and O2 records the alternative for the owner.
2. **The existing mediahost test could not survive the fence unchanged.**
   `idle_reset_lands_without_pcm_and_clears_history_before_the_next_block`
   called `session.reset()` twice around a held first callback — under the
   fence the second call deadlocks against the held first (the pump cannot
   ack before the callback returns). Restructured: the first reset runs on a
   helper thread, the fence is asserted with `!handle.is_finished()` while
   the callback is held, and the wait's return is joined after release.
3. **The "second reset queued while the first callback is held" sub-case
   moved level.** Under the fence the session serializes resets by design
   (`&mut self`, one outstanding reset), so a burst exists only below the
   session. The coverage moved to a raw-transport host-side test
   (`queued_resets_are_each_acknowledged_in_order_while_the_host_is_busy`),
   which pins one-ack-per-reset, in order, through `poll_commands`' queue
   drain.
4. **The hand-off's "optionally" for the plughost array was done** (the task
   says every matcher), plus the module-doc mention.
5. **`command()` gained an explicit arm** (not named by the hand-off) so a
   host-side ack fails with the direction rule rather than the generic
   catch-all.
6. **Variant placement has a wire cost the hand-off did not spell out:**
   inserting beside `DspReset` moves `DspError` from index 26 to 27. Safe
   under the `Hello` gate (a v3 peer never reaches the conversation); stated
   in the RFC (§2) with the append-at-end alternative recorded as O3.

Verified compatible without edits (read-only): `rustamp-audio`'s fake
resident runs the production `serve_dsp` (`resident.rs:2173`), so it
acknowledges automatically and the adapter/engine tests — including the A26
red `engine_gapless_seam_through_the_resident_chain_is_exact` — need no
change; `ChainControl::reset` checks `poisoned` first (`resident.rs:836`),
so the wedge/death tests never hit a fenced reset; `rustamp-dsphost`'s
`reset_keeps_rings_and_continues` gets its ack from the same pump. The whole
workspace (all targets, all features) compiles with `-D warnings`.

Note for the audio lane (not touched here): the doc comments at
`crates/rustamp-audio/src/resident.rs:823-834` and `:2249-2254` describe the
pre-fence semantics ("no ordering fence", "re-send the reset and re-probe")
and become stale once the fence lands; the tests themselves are correct as
written (the probe loop's first iteration now succeeds). Rewording them is
the audio lane's call.

## 8. What only the VM can prove

- The A26 red goes green: `cargo test -p rustamp-audio --all-features` in the
  guest, in particular
  `resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact`
  (and the first-track-start race it shares). Per M26 §5 the bar is **at least
  5 consecutive green full-suite runs** with the previously-red binary green
  in every one (the race is probabilistic), plus 2-3 repeats of the
  `rustamp_audio` lib binary.
- The new mediahost tests: the fence tests (the `try_recv`-when-returns
  assertions and the `!is_finished` hold), the raw-transport burst test, and
  the timeout test (a real 5 s `CONTROL_WAIT` expiry on the guest scheduler,
  then the sticky-failure and late-ack assertions).
- That the fenced reset's bounded wait always suffices on the guest's
  scheduler (the pump polls commands every 20 ms; the bound is ~250 slices of
  margin) and that the adapter-level probe tests and the seek test stay green
  with the fenced reset.
- `rustamp-dsphost`'s `reset_keeps_rings_and_continues` and the rest of the
  `rustamp-mediahost`/`rustamp-dsphost` suites: no regression.
- The `Hello` gate against a v3 helper (mixed-version pair rejected cleanly):
  covered by inspection of `control.rs:158-166` and the app's barrier; a real
  mixed pair needs two binaries, which is a packaging/VM item, not a unit
  test.

## 9. Files and line counts

Counts include blank lines; all inside the owned paths. `git diff --stat`:
385 insertions, 60 deletions across 7 modified files, plus 2 new files.

| File | Lines (after) | Change |
|---|---:|---|
| `docs/rfc/0015-dsp-reset-acknowledgement.md` | 218 | New: the RFC (Proposed). |
| `crates/rustamp-ipc/src/message.rs` | 290 | +12/−3: `DspResetAck` beside `DspReset`; `PROTOCOL_VERSION` 3 → 4 with the history entry. |
| `crates/rustamp-ipc/tests/hosting_wire.rs` | 495 | +63/−: version 4; ack round-trip at 0x1a; `DspError` at 0x1b; 28 variants; sequential indices 15..=27. |
| `crates/rustamp-ipc/tests/command_wire.rs` | 413 | +13/−: version 4 test renamed and updated. |
| `crates/rustamp-mediahost/src/dsp.rs` | 876 | +69/−: the ack at both application sites (pump + post-EOS idle loop), the explicit `command()` arm, the fenced `DspSession::reset` with the sticky timeout, doc rewrites. |
| `crates/rustamp-mediahost/src/lib.rs` | 216 | +5: the stub `DspHost`'s explicit `DspResetAck` arm. |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 493 | +274/−: restructured idle-reset test with fence assertions; new fence, burst and timeout tests; file docs. |
| `crates/rustamp-plughost/src/control.rs` | 438 | +9/−: `DspResetAck` in the foreign-message array; module-doc mention. |
| `docs/tasks/M27-reset-fence.md` | this card | New. |

## 10. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0; one formatting fix applied
  with `cargo fmt --all` on the way — a line-length wrap in `dsp.rs`).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited target plus all test
  targets of every workspace crate — nothing was executed). Three findings
  fixed on the way: a missing `PcmRings` trait import and a missing `mut`
  binding in the new test, plus the formatting wrap.
- `cargo build --workspace --offline`: **PASS** (exit 0, 45 s).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned beyond the toolchain's own
  compile steps, no download.

## 11. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the files
above and nothing else:

```
 M crates/rustamp-ipc/src/message.rs
 M crates/rustamp-ipc/tests/command_wire.rs
 M crates/rustamp-ipc/tests/hosting_wire.rs
 M crates/rustamp-mediahost/src/dsp.rs
 M crates/rustamp-mediahost/src/lib.rs
 M crates/rustamp-mediahost/tests/dsp_control.rs
 M crates/rustamp-plughost/src/control.rs
?? docs/rfc/0015-dsp-reset-acknowledgement.md
?? docs/tasks/M27-reset-fence.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written. The A26 card was read from the
sibling worktree `wave77-gapless-real` (its owning lane's worktree), which is
not one of the forbidden trees. No file outside the owned paths was modified.

## 12. Provenance

Implemented only from the task cards (A26 — read from `wave77-gapless-real`,
A22, M19, M26), the RFCs they cite (RFC 0003 §§8-9, RFC 0005 §12, RFC 0006
§§4, 6.2, 7, 8, RFC 0007), the RFCs read for shape (0006, 0007, 0012, 0014),
and the clean repository code (read-only outside the owned paths). No Winamp
source, SDK headers, disassembly or forbidden-tree contents were read. No
third-party code copied. License impact: none — no dependency change;
`rustamp-ipc`, `rustamp-mediahost` and `rustamp-plughost` stay
MIT OR Apache-2.0 (RFC 0004's permissive set is unchanged; the new variant is
a unit variant, no new wire type).
