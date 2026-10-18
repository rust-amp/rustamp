# M23: Green the two real-app reds the VM named (wave 72)

**Date:** 2026-10-18. **Lane:** `wave72-real-app`.
**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave72-real-app`.

## Boundary and scope

Write only inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

Owned paths: `crates/rustamp-dechost/**`, `crates/rustamp-app/**`, and this
card. Touched: `crates/rustamp-dechost/tests/real_decode.rs`,
`crates/rustamp-app/src/app.rs`, and this card. `crates/rustamp-audio`,
`crates/rustamp-dsp`, `crates/rustamp-mediahost`, `crates/rustamp-ui`,
`crates/rustamp-core`, `crates/rustamp-decoders`, `crates/rustamp-playlist`
and every other crate were **not** touched — in particular **no `src/`
behaviour was changed anywhere; both fixes are test-surface corrections to
expectations the VM proved wrong.**

Static only: **no `cargo test`** — not the suite, not `--no-run`, not a
single binary. No test process, example, player, helper binary or reference
binary was spawned; no VM was touched; no window or audio device was opened;
no download, no dependency addition, no install, no commit and no other git
write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §6). Every command ran
inside this worktree.

Sources: the M12/M22 cards, the N7/N8/N9/N11 cards, RFCs 0002/0003/0006,
the M10/M14/M17/M18 cards, and the existing clean repository code only (read
across crates where the diagnosis needed it). No Winamp source, SDK material
or disassembly was read; no third-party code was copied.

The VM record this lane greens: the two failures the wave-72 run named —
`garbage_and_empty_files_are_clean_errors_never_eos` in the decoder
resident's suite (`crates/rustamp-dechost/tests/real_decode.rs`) and
`app::tests::good_playlist_file_loads_and_plays` in the app crate
(`crates/rustamp-app/src/app.rs`). Both verdicts below are static proofs
with `file:line`; the VM run remains the execution proof.

---

## 1. Decoder failure — the exact variant was pinned to a mapping the framework does not deliver

**Test:** `garbage_and_empty_files_are_clean_errors_never_eos`
(`crates/rustamp-dechost/tests/real_decode.rs:338`; the failing assertion was
at `:354-357`).

**What the printed outcome says.** The test's `described` mechanism (added
so a failure names the variant instead of only the property,
`real_decode.rs:360-363`) prints the outcome it saw. Traced statically
through the current tree, the failure message is:

```
garbage.bin: undecodable input must be a loud error, never EOS
(got Err(Disconnected("resident decoder failed: operation failed: unsupported format: bin")))
```

and for the second file (`empty.wav`):

```
Err(Disconnected("resident decoder failed: operation failed: decode error: empty source: 0 bytes to decode"))
```

So the different error kind is **`Disconnected`**, wrapping
`"resident decoder failed: operation failed: <core reason>"` — not the
`OperationFailed` the assertion pinned.

**Where that comes from (the full chain, file:line):**

1. The resident refuses loudly with an honest reason:
   `RegistryResident::open` maps every registry open failure to
   `MediaHostError::OperationFailed(error.to_string())`
   (`crates/rustamp-dechost/src/lib.rs:117-119`). For `garbage.bin` the
   registry finds no claiming factory and returns
   `Error::UnsupportedFormat("bin")` (`crates/rustamp-core/src/audio.rs:301-306`);
   for the 0-byte `empty.wav` symphonia reports
   `Error::Decode("empty source: 0 bytes to decode")`
   (`crates/rustamp-decoders/src/symphonia_dec.rs:303-305`).
2. `serve_decoder` publishes that failure as a terminal `DecoderError`
   (`crates/rustamp-mediahost/src/decoder.rs:160-168`).
3. The player's control monitor **consumes** `DecoderError` and fails the
   peer sticky — `fail("resident decoder failed: {message}")`, then breaks
   (`crates/rustamp-mediahost/src/control.rs:183-186`, there since the M10
   framework). It never forwards the message to the session's incoming queue.
4. `DecoderSession::open` therefore never reaches its own
   `Message::DecoderError => OperationFailed` arm
   (`crates/rustamp-mediahost/src/decoder.rs:316-318`) — **that arm is
   unreachable dead code**; instead `peer.recv`'s sticky-failure check
   returns `Disconnected("resident decoder failed: …")`
   (`control.rs:243-248`, `274-287`).

**Verdict: the test expectation was too narrow; the code is right.** The
exact-variant pin (`matches!(outcome, Err(MediaHostError::OperationFailed(_)))`)
asserted a mapping the framework cannot deliver: the session's
`DecoderError → OperationFailed` arm is dead because the monitor eats the
message first. Is `Disconnected` the honest kind? The *kind* is the
framework's deliberate, load-bearing design — a resident's terminal error
fails the peer sticky so that blocked PCM reads wake and a dead host is
never mistaken for EOS (the M10/M17 design; the same monitor arm also
handles `DspError`). The *honesty* lives in the message, which carries the
resident's full refusal reason verbatim. The framework's own
resident-failure test pins exactly that contract and deliberately no
variant: `decoder_error_is_loud_and_never_eos`
(`crates/rustamp-mediahost/tests/pcm_transfer.rs:343-365`) asserts an `Err`
whose string contains the resident's reason, never `Ok(0)`, and a host that
ends `Err`.

**Why the broader claim is the right one (not the narrower `Disconnected`
pin):** the variant is another crate's internal mapping — the mediahost
lane's monitor — and exists to wake blocked PCM workers. Pinning it here
would couple this crate's test to a mapping the resident does not control
and would fail on a legitimate framework change (for example if the
session's dead `DecoderError` arm is ever made reachable, the player would
see `OperationFailed` and this test would break although the resident's
behaviour is unchanged). The broader claim keeps everything the test
always wanted to prove, and the reason pin keeps it strong: the error must
carry the resident's own refusal marker (`"operation failed"` — the
`Display` prefix of the `OperationFailed` the resident itself constructs at
`lib.rs:117-119`, preserved verbatim through the framework's wrapping), so
a bare transport failure (`TimedOut`, a reason-less `Disconnected`) or an
accepted session can never pass.

**Fix (test only, `real_decode.rs:350-370`).** The assertion now requires:
the outcome is an `Err` (never `Ok` — the "the undecodable file was
accepted" panic path is kept), and the error's `Display` contains
`"operation failed"` (the resident's refusal reason survives the wrap). The
`described` printing is kept in both failure messages. The host-side
assertion is unchanged: the host thread must end with `Err`, never stall,
never `Ok`. Nothing was weakened: the loud-error/never-EOS claim, the
host-ends-`Err` claim and the reason content are all still pinned, and the
variant pin is replaced by a *stronger* reason pin.

**Follow-up (not owned here, not edited):** the M12 card's table row
(`docs/tasks/M12-decoder-resident.md:103`) still says the player sees
`OperationFailed` for garbage/empty files — that was written unexecuted and
is wrong about the variant; the resident does return `OperationFailed`, the
player sees `Disconnected` carrying it. Also for the mediahost lane: the
unreachable `Message::DecoderError` arms in `DecoderSession::open`
(`decoder.rs:316-318`) and `DecoderSession::remember` (`decoder.rs:396-399`)
are dead code because the monitor never forwards `DecoderError`; either
remove them or forward-then-fail, so the next reader does not pin the dead
arm again.

---

## 2. App failure — the test asserted a UI state no pump had delivered

**Test:** `good_playlist_file_loads_and_plays`
(`crates/rustamp-app/src/app.rs:3569`; the failing assertion is
`assert_eq!(app.state().transport, TransportState::Playing)` at `:3614`,
preceded by the `pump_until` wait at `:3607-3613`).

**History (the test predates the lanes that landed after it).** The test was
written by the load-playlist lane N7 (`9e8c5d0`, "never executed" per the N7
card and N8's card) and is unchanged since. The app crate then took two
more lanes — the red-tests lane N8 (`24b33e4`) and the configuration-class
lane N9 (`99b2cb3`) — and this test predates both. Neither is the cause:
`git log 24b33e4..HEAD` is empty for `crates/rustamp-audio/src/engine.rs`,
`crates/rustamp-ui/src/behaviour.rs`, `crates/rustamp-ui/src/state.rs`,
`crates/rustamp-playlist/src` and for this test's own code, so the engine's
play path and the UI's event handling are exactly what they were before
those lanes. The defect is in the N7 test itself, latent until the VM first
executed it: it has been the app crate's second red ever since N7 merged
(the unnamed second failure N11 could not isolate —
`docs/tasks/N11-app-platform-red2.md` §3 — now named by this run).

**Why it fails (the chain, file:line):**

1. `app.dialog_key(Key::Enter)` confirms synchronously:
   `confirm_load_playlist` → `App::load_playlist_file(&path, true)`
   (`crates/rustamp-app/src/dialogs.rs:224-239`) →
   `self.engine.play_index(0)` (`app.rs:1492-1497`).
2. `Engine::play_index` → `start_run` sets `self.transport = Playing` and
   publishes `Event::StateChanged(Playing)` **on the calling thread**
   (`crates/rustamp-audio/src/engine.rs:1412-1413`; `play_index` at
   `:834-843`). So when `dialog_key` returns, the engine is already
   `Playing` with `current_index() == Some(0)`, and the event sits in the
   bus **undelivered**.
3. `pump_until` checks its condition **before the first pump**
   (`app.rs:1748-1762`, the `if cond(app) { return true; }` at `:1749-1751`).
   The condition watches only the engine (`app.rs:3609-3610`), so it is
   already true and the wait returns having pumped **zero** times.
4. The UI machine's transport is set only from a pumped
   `Event::StateChanged` (`App::pump` drains the bus at `app.rs:689-692` →
   `handle_event` → `ui.handle_event` → `set_transport`,
   `crates/rustamp-ui/src/behaviour.rs:233,376-384`) or from a Play/Pause
   command prediction — and the playlist load issues no Play command. So
   `app.state().transport` is still `Stopped` when asserted at `:3614`.

The failure is deterministic, not a load flake: nothing in the sequence
delivers the event before the assert.

**Verdict: the test expectation was wrong; the code is right.** The
architecture is explicit — the engine is authoritative and the UI confirms
through events (`behaviour.rs:280-284`, "the core remains authoritative and
confirms through `Event`s"); the app's real event loop pumps continuously,
so the UI state converges within one pump. The test asserted the converged
state without pumping once. The sibling chain test written by the earlier
N3 lane shows the correct shape:
`clicking_play_turns_the_ui_outcome_into_a_command_the_engine_obeys`
(`app.rs:1811-1818`) pumps until `app.state().transport == Playing` — the
UI state itself — and the sibling playlist test
(`playlist_file_with_bad_entry_loads_surviving_and_reports_skips`,
`app.rs:3624+`) passes precisely because it asserts no UI state.

**Fix (test only, `app.rs:3601-3614`).** The `pump_until` condition now
also requires `app.state().transport == TransportState::Playing`, so the
wait actually waits for the event it asserts: the initial check is false
(UI still `Stopped`), the first pump delivers the already-published
`StateChanged(Playing)`, and the condition converges. No claim was weakened
— the engine-side claims (`transport() == Playing`,
`current_index() == Some(0)`) and the UI-side claim are all still pinned,
and the trailing `assert_eq!` is kept; the test now proves the UI *shows*
Playing rather than assuming it.

---

## 3. Assertions kept (none weakened)

- Decoder: 256 junk bytes and a 0-byte file are both refused at `open`
  with a loud error that carries the resident's refusal reason — never an
  accepted session, never EOS; the host thread ends with `Err` within
  `JOIN_WAIT`, never stalls, never `Ok`. The variant is no longer pinned
  (it never was the framework's to pin here); the reason is pinned instead,
  which is the stronger claim.
- App: the dialog opens, the staged file loads, the queue receives both
  entries in order with the resolved paths, the playlist model syncs to
  two entries, playback starts on track 0 (engine transport `Playing`,
  current index 0) **and** the UI machine shows `Playing`, the report is
  clean with two entries and zero skips, and no error is recorded.

## 4. Gate receipts

- `cargo fmt --all -- --check`: **exit 0**.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, no warnings (compiles both edited test targets).
- `cargo build --workspace --offline`: **exit 0**.
- **`cargo test` was NOT run** — not the suite, not `--no-run`, not a single
  binary. No test process, example, player, helper binary or reference
  binary was spawned; no VM was touched; no window or audio device was
  opened; no download, no dependency addition, no install, no commit and no
  other git write. Every command ran inside this worktree.

## 5. Files and line counts

Counts include blank lines; all inside the owned paths.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-dechost/tests/real_decode.rs` | 441 | modified (was 428): the garbage/empty assertion now pins the loud refusal with the resident's reason instead of the unreachable `OperationFailed` variant |
| `crates/rustamp-app/src/app.rs` | 3758 | modified (was 3752): the playback wait now also covers the UI state it asserts |
| `docs/tasks/M23-real-app-red.md` | this card | new |

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file under `crates/rustamp-audio`, `crates/rustamp-dsp`,
`crates/rustamp-mediahost`, `crates/rustamp-ui`, `crates/rustamp-core`,
`crates/rustamp-decoders`, `crates/rustamp-playlist` or any other crate was
touched.

## 6. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the two
modified files above plus this card — nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written,
and no file outside the owned paths was modified.

## 7. Provenance

- Implemented only from the task cards, RFCs and clean-room Rustamp
  repository code (read-only outside the owned paths). No Winamp source, SDK
  headers, disassembly or forbidden-tree contents were read. No
  third-party code copied. License impact: none (no dependency change;
  both crates keep their GPL-3.0-or-later license).
- What only the VM can prove: both fixed tests green in the Hyper-V suite
  (`real_decode` 7/7, the app crate's remaining red gone), and no
  neighbouring red introduced.
