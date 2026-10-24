# X9 — smoke hash leg: the manifest hashed the transcript before the transcript was complete

**Date:** 2026-10-18. **Status:** guest script + host runner + docs changed,
VM not run. **Worktree / write boundary:** `wave84-smoke-hash` worktree only.
Owned paths: `docs/notes/vm-broker/guest/run-smoke.ps1`,
`docs/notes/vm-run-smoke.ps1`, `docs/tasks/BETA-smoke.md`, this card. No
crate, test runner or broker script touched; no download, no new dependency,
no git write, no window, no audio.

## 1. The failure that motivated this

The ninth packaged-player smoke run got all the way through the guest —
interactive session, main window `Rustamp` at client 275x116, live bundled
host child, capture with its PNG gates, the silent marker, the stop with its
exit recorded — and the proof was copied back. Then the host's final step
said, verbatim:

```text
--- verifying hashes (byte-identical or refused)
STEP verify FAILED: hash mismatch on 'guest-transcript.txt': guest <hash> host <hash>
```

Every guest check had passed; only the transcript's hash failed. The check
was working exactly as designed — the manifest was describing a file that
no longer existed.

## 2. The write order in both halves (file:line, before this lane's edit)

Line numbers are those of the file the ninth run used. The inner payload is
the here-string written to `inner.ps1` (`run-smoke.ps1:132-490`); for that
file **inner.ps1 line = run-smoke.ps1 line − 132** (content starts at file
line 133).

| # | file:line (inner line) | what happens |
|---|---|---|
| 1 | `run-smoke.ps1:143` (11) | the payload binds `$Transcript` to `C:\rustamp\out\smoke\guest-transcript.txt` |
| 2 | `:145-146` (13-14) | `$log` + `Say`: every step appends a line **in memory**; the file is not touched yet |
| 3 | `:406-407` (274-275) | `Say 'player up (s)'` — the last line before the stop |
| 4 | `:408` (276) | `$player.Kill()` |
| 5 | `:410` (278) | **`Say 'player exit code' $player.ExitCode`** — the stop record and the exit code enter the log |
| 6 | `:411-412` (279-280) | `player.stdout.log` / `player.stderr.log` written |
| 7 | `:414-416` (282-284) | silent-path marker read and asserted |
| 8 | `:420-424` (288-292) | `Say 'host reaped with player'` |
| 9 | `:448` (316) | `proof.json` written |
| 10 | `:449` (317) | **the transcript is written** — the payload's last write to it (stop record and exit code included) |
| 11 | `:450-456` (318-324) | **the hash manifest `hashes.json` is written** — it hashes every file under the output directory except itself, **including `guest-transcript.txt` exactly as it stands at step 10** |
| 12 | `:457` (325) | the `done.json` sentinel is written — after the manifest, so never covered by it |
| 13 | `:516-522` | the outer half (broker's session-0 session) waits for the sentinel |
| 14 | `:526` | the outer half reads `proof.json` |
| 15 | `:532-541` | the outer half's trust checks; any failure **throws here**, so what follows runs only on a fully successful payload |
| 16 | `:546-550` | **the outer half appends to the same `guest-transcript.txt`**: a blank line (`:548`) plus its nine session-discovery entries (`:549` — `launcher session`, `launcher user`, `player`, `bundled host`, `capture tool`, `fixture`, `interactive session`, `inner payload`, `smoke task`, collected at `:77-78`, `:88-91`, `:108`/`:110`, `:493`, `:510`). **This is the last write to the transcript — and it happens after the manifest hashed it** |
| 17 | `vm-run-smoke.ps1:236` | the host's `copyout` brings `C:\rustamp\out\smoke` back — with the appended transcript |
| 18 | `vm-run-smoke.ps1:265-268` | the host reads `hashes.json` |
| 19 | `vm-run-smoke.ps1:270-276` | the host recomputes every entry (`:273`) and compares (`:274`) — the mismatch line above |

**Exactly which lines are written after the manifest:** the transcript
gains ten lines the manifest never saw — one blank line plus the nine
outer-half entries listed at step 16. The manifest's `guest-transcript.txt`
entry is therefore a hash of a strict prefix of the file that comes back.

Failure paths, for completeness: `Finish-Failure` (`:157`, inner 25) and
the catch leg (`:477`, inner 345) write the transcript but **no manifest**,
and the outer half throws at `:541` before its append — so a failed guest
step brings back an inner-only transcript with no `hashes.json`, the host
fails at `STEP guest` first, and the hash leg only ever runs on a run that
already passed every guest check.

## 3. Why the check could never pass

Structural, not corruption. The manifest was written at step 11 over a
transcript that step 16 was guaranteed to extend: the outer half's append
sits after the payload's exit, unconditionally, on every successful run.
The guest hash is the hash of the inner-only transcript (through `host
reaped with player`); the host hash is of the appended file. No successful
run can ever satisfy it — the check failed on success, on the same file,
every time. Every other manifest entry (`proof.json`, `player.png`,
`window-list.log`, `capture.log`, `player.stdout.log`,
`player.stderr.log`, `inner.ps1`, and `rustamp.toml` when the player wrote
one) is final before the manifest and verified — which is exactly the
observed failure: one mismatch, on the proof's own transcript, after
everything else passed.

The capture lane does not share the defect: `run-capture.ps1` writes its
transcript once (`:188`) and hashes afterwards (`:192-197`) in the same
half, with no second appender (`docs/notes/vm-capture-runbook.md` §4). The
smoke lane diverged from that shape by letting two halves share one live
log — the manifest simply stayed in the first half.

## 4. The fix, and why this option

Three repairs were available:

- **Exclude the transcript from the byte-identical set**, recorded in the
  manifest with a reason. **Rejected.** The transcript is the proof's
  primary evidence — window facts, capture gates, the stop record, the exit
  code, the silent marker — and it *can* be byte-verified; the mismatch was
  an ordering bug, not a property of the file. Excluding it would leave the
  most important artifact unverified while the step still says "every file
  recomputed".
- **Split the transcript** into a finished part and a live log.
  **Rejected.** It verifies only the payload's half of the transcript and
  demotes the outer session-discovery trail — which the script's own
  comment calls the number-one historical failure point — to an unverified
  log; it also replaces the one transcript the owner reads with two files.
  Honest, but strictly weaker than the option below, which is available
  precisely because the transcript does become complete.
- **Hash the transcript after it is complete. Chosen.** Concretely: the
  manifest write moves from the inner payload (`:450-456`) to the outer
  half, immediately after its append (`:546-550`). The manifest is then
  written **once, after the last write to any file it covers** — the
  invariant "a file whose hash is checked is a file the check can actually
  verify" holds structurally, not by post-hoc patching. The completed
  transcript — stop record and exit code included, plus the
  session-discovery trail — stays **inside** the byte-identical set, and
  the manifest now also covers the `done.json` sentinel (final by then;
  previously it postdated the manifest and was uncovered).

Why this keeps the proof honest: before the fix, the transcript's manifest
entry was a hash of a file that no longer existed — unfalsifiable in the
wrong direction, failing on success. After the fix, the manifest is the
last write in the guest, every entry is final when hashed, the host
recomputes every entry as before, and the host additionally **refuses a
manifest that does not cover `guest-transcript.txt`**
(`vm-run-smoke.ps1:280-282` after this lane), so a future regression that
drops the transcript from the byte-identical set fails loudly instead of
verifying a subset. The check is stricter than before, not weaker; nothing
was removed.

## 5. What changed

`docs/notes/vm-broker/guest/run-smoke.ps1`:

- Inner payload: the manifest write is removed from the finalize step
  (now `proof, transcript, sentinel`); a comment states why no manifest may
  be written there.
- Outer half: after appending its trail to the transcript, writes
  `hashes.json` over the final output tree — every file under
  `C:\rustamp\out\smoke` except `hashes.json` itself (the same loop the
  payload used, moved verbatim).
- Header comment: the manifest's write point and the reason are stated up
  front.
- After the edit: payload finalize at `:432-463` (transcript write `:461`,
  sentinel `:462`), outer append `:551-555`, manifest block `:557-573`,
  single-object output from `:578`. Inner mapping after the edit:
  inner.ps1 line = file line − 138.

`docs/notes/vm-run-smoke.ps1`:

- No functional change to the recompute loop. New assertion before it: a
  manifest without a `guest-transcript.txt` entry fails `STEP verify`
  (`:280-282`).
- The verify-step comment states the new invariant (manifest written after
  the transcript is complete, every entry verifiable).
- The failure-path copyout comment is corrected: on a failed guest step
  the transcript is the payload's only (the outer half appends and writes
  the manifest only on success), so a failed run brings back no manifest
  and the hash leg never runs for it.

`docs/tasks/BETA-smoke.md`: defect (8) of nine runs recorded; the `verify`
row states that the completed transcript is byte-verified and that an
uncovered transcript is refused.

## 6. What is verified byte for byte now, and what is not

The guest manifest covers **every file in `C:\rustamp\out\smoke` except
`hashes.json` itself**, and the host recomputes every entry:

- `guest-transcript.txt` — the **completed** transcript: the payload's
  trail (session, screen, window facts, capture gates, `player exit code`,
  silent marker, host-reaped line) plus the outer half's session-discovery
  trail.
- `proof.json`, `player.png` (the captured image — byte-identical, on top
  of the unchanged PNG-signature and ≥512-byte gates), `window-list.log`,
  `capture.log`, `player.stdout.log`, `player.stderr.log`, `inner.ps1`
  (the payload itself), `done.json` (newly covered), and `rustamp.toml`
  when the player wrote one.

Not verified byte for byte, named honestly:

- `hashes.json` itself — a manifest cannot cover itself (same convention as
  the `vm-run-capture.ps1` §7 leg).
- The staged **input** tree `C:\rustamp\smoke` (`player\rustamp.exe`,
  `player\rustamp-plughost32.exe`, `tools\xtask.exe` + CRT DLLs,
  `media\sine.flac`). The proof claims these are verbatim copies of the
  audited package files (host audit hashes `vm-run-smoke.ps1:78-86`,
  verbatim `Copy-Item` at `:129-137`) — unchanged by this lane — but
  nothing re-hashes them inside the guest. That residual gap predates
  this lane and is recorded here rather than silently inherited.
- Still not claimed, unchanged (`BETA-smoke.md` "What it still does not
  prove"): the PNG's pixels (BETA-02 placeholder art), which `Rustamp*`
  window the PNG shows, the pipe bytes behind the live-host-child
  handshake, a real audio endpoint, a clean exit, decode beyond the one
  fixture.

## 7. What did NOT change (smoke properties kept)

- The package is staged byte for byte and never rebuilt; only
  `cargo build -p xtask` runs, on the host.
- The player runs with `--no-audio` (null audio device) and a real window
  in the guest's interactive session; the client-size check is still
  275x116; the window dump on failure (X5) is unchanged.
- The live bundled-host child is still required (~10 s).
- The capture still goes through `xtask capture` with the exit-code gate,
  the PNG-signature gate and the ≥512-byte gate; the tool's stderr notes
  are still tolerated, not ignored (X7).
- The silent-path stdout marker is still required.
- The stop is still `Kill` + `WaitForExit(10000)` with the exit code
  recorded in the transcript — and that record is now inside the
  byte-verified set.
- The host's hash recompute is unchanged and now also refuses a manifest
  that omits the transcript. No check was weakened into "compare nothing".

## 8. Still open (not this lane's scope)

- The payload's failure paths still exit without killing the player
  (X6 §7.4, X7 §8): after an inner failure the player lives to its
  `--seconds 120` backstop and can lock the staged host binary for the
  next run's copy-in. Remedy: `-Reset`.
- The staged input binaries are not re-hashed inside the guest (§6). A
  guest-side input audit would be a new step in `run-smoke.ps1`; it is
  named here, not implemented here.
- The host's transcript-coverage assertion hardcodes
  `guest-transcript.txt`; renaming the transcript means changing the
  guest's manifest writer and the host's assertion together.

## 9. Verification and boundary receipt

- PowerShell parser (`[System.Management.Automation.Language.Parser]::ParseFile`),
  error counts, each under **both** the PowerShell 7.6.6 parser and the
  Windows PowerShell 5.1.26100 parser:
  - `docs/notes/vm-broker/guest/run-smoke.ps1` — **0 errors**, 597 lines
    (574 before this lane);
  - `docs/notes/vm-run-smoke.ps1` — **0 errors**, 298 lines (284 before);
  - the extracted inner payload (the here-string content, written to
    `inner.ps1` by the script) — **0 errors**, 356 lines, parsed as a
    standalone script because this lane edited inside the here-string.
  - `docs/tasks/BETA-smoke.md` — 215 lines (198 before).
- No VM, broker, player, host, capture tool, test binary or reference
  binary was run; no window opened; no audio; no download, install, commit
  or git write; no new dependency. The Rust gates (`cargo fmt` / `clippy` /
  `test`) were not run: no crate, test runner or broker file was touched,
  and this lane's verification scope is the parser check.
- Provenance: clean-tree sources only (the two scripts, `BETA-smoke.md`,
  the X5-X8 cards, `docs/notes/vm-capture-runbook.md` §4,
  `docs/notes/vm-broker/guest/run-capture.ps1`,
  `docs/notes/vm-run-capture.ps1` §7, the ninth run's failure line). No
  Winamp source, SDK header, disassembly or dirty-room material read or
  touched.
- `git status --short` (worktree): exactly the owned files —
  `docs/notes/vm-broker/guest/run-smoke.ps1`, `docs/notes/vm-run-smoke.ps1`
  and `docs/tasks/BETA-smoke.md` modified, `docs/tasks/X9-smoke-hash.md`
  new; no tracked modification outside them. The main checkout
  `D:\code\Rustamp` received no write from this lane: its git status shows
  no tracked modification, only the harness's own untracked
  `opencode.local.json` (an opencode config file; no write of this lane
  targeted the main checkout). `D:\code\rustamp-research` and `D:\code\rustamp-refbin` were
  only probed with `Test-Path`, never read or written.
