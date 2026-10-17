# M16: Contract RFC for loud mid-run DSP errors (F1 follow-up)

## Boundary and scope

Write only `docs/rfc/0014-fallible-dsp-stage.md` and this card in the
assigned worktree. Nothing under `D:\code\Rustamp` was read or written;
no `D:\code\rustamp-research` or `D:\code\rustamp-refbin` material was
touched (not even read); no dirty-room/reference material of any kind.
Sources: the in-tree code listed below plus RFCs 0001, 0002, 0006,
0012, 0013 and cards M13/M14 only. No copied third-party code, no
downloads, no new dependencies, no commits. No code was implemented:
this lane drafts the contract for the owner to accept or reject.

One deliberate exception to "owned paths only", mirroring M13/M14: the
assigned worktree did not exist when this lane started, so it was
created as a detached checkout of `main` (`git worktree add --detach`
at `db461c6`, the same shape as the sibling wave worktrees). No branch,
commit or content change came with that; see §6.

## 1. Why this card exists

M14 finding F1 (Medium, PROVEN) showed the DSP boundary cannot report
a mid-run fault with diagnostic text: `DspStage::process` returns `()`
(`crates/rustamp-core/src/audio.rs:401`), and the resident mirror
`ResidentChain::process` returns `()` too
(`crates/rustamp-mediahost/src/dsp.rs:39`). The decoder boundary is
fallible per block and every failure becomes text; the DSP boundary can
only fail at `open` or by panicking (text lost). M14 states the fix
"needs a core-contract RFC", and the review prompt for this lane adds
that the hole mirrors the core `DspStage::process` contract, so the
decision is a core-contract one. RFC 0014 is that draft.

## 2. What the RFC decides (draft, owner to accept or reject)

- **Recommended: fallible `process`** — `DspStage::process` and
  `ResidentChain::process` return `Result<()>`. The engine treats a
  stage error like a decoder error (`report_error`, end the track);
  the resident pump propagates with `?` into the *existing* `DspError`
  send, so the player hears the text with no wire change and
  `PROTOCOL_VERSION` stays 3.
- **Considered and rejected:** an error latch plus polled accessor in
  the style of RFC 0012 (non-breaking but detaches the error from the
  failing call and doubles the contract surface; the audio-thread
  constraint that forced RFC 0012's choice does not apply here since
  `process` runs on threads that propagate `Result`), and keeping
  `process` infallible (documents the hole instead of closing it).
- **The rule:** `Err` is terminal for the run; first failure sticks;
  a failed chain/resident is poisoned (reconfigure or drop, no
  resume); panics stay bugs; the engine's NaN sanitizer stays as
  defense in depth on the Ok path.
- **Proposed alongside:** `ErrorKind::Dsp` appended at the end of the
  proto enum (the RFC 0002 C4 maneuver, existing indices stable) with
  `Error::Dsp` mapped to it instead of `Other`; `Error::Dsp`'s doc
  widened from configuration-time to mid-run faults.
- **Explicitly unchanged:** the wire (no `Message` diff), the
  two-ring handshake, generations, drain-then-sticky-EOS,
  `DspReset`-never-closes-rings, `prepare`/`reset`/`latency_frames`,
  licensing and dependencies.

## 3. Call-site breakage (all verified in-tree, all in the RFC §6)

No out-of-tree implementer can exist yet: classic `dsp_` bridging is
future work (M13 §5), and the `plughost` DSP symbols are host callbacks
for classic plugins (`loader.rs:152-159,488-489`), not a `DspStage`
adapter — so no plugin-bridge call site breaks.

- Core: `audio.rs:401` (decl), `error.rs:30-32` + `:65-72` (variant
  doc, kind mapping), `mock.rs:248` (impl), `mock.rs:324` (test).
- Engine: `decode.rs:639` (`emit`, the one production behavior
  change) + `:921-927` (`report_error`, reused as-is),
  `engine.rs:308` (signature unchanged), `tests/engine.rs:737,1417,1772`
  (three test doubles).
- Resident: `mediahost/src/dsp.rs:39` (trait) + `:184` (pump, the one
  framework behavior change); `dsphost/src/lib.rs:67,95` (stage impls),
  `:158` (`Chain`), `:296,306,326` (tests).
- Stages: `dsp/src/eq.rs:429` + 15 test call sites,
  `dsp/src/limiter.rs:316` + 8 test call sites,
  `audio/tests/limiter_gain.rs:59`.

## 4. What is left for the owner (RFC §8, undecidable from the code)

- **Q1 — stop vs bypass:** should a mid-run stage error end the track
  (damage, like a decode error) or bypass the stage (telemetry, like
  `skipped_packets`)? Needs a product decision; recommendation is stop.
- **Q2 — `ErrorKind::Dsp` vs `Other`:** needs to know whether the UI,
  plugin guide or IPC surface must distinguish DSP faults. Either way
  the message text already names them.
- **Q3 — resident restart policy:** explicit caller reconfigure vs a
  session convenience re-open; needs packaging/UX input. Default is
  explicit, no auto-retry (cf. RFC 0012 Q1 fail-to-stop).
- **Q4 — out-of-tree implementers:** only the owner can confirm none
  exist, which is what makes the breaking option cheap. Fallback if
  they do: additive `try_process` with a default body.

Nothing was implemented; the implementing lane starts after the owner
accepts the RFC (acceptance criteria are RFC §7).

## 5. Files and line counts

Line counts via `(Get-Content).Count` (content lines, no diffstat).

| File | Lines | Change |
|---|---:|---|
| `docs/rfc/0014-fallible-dsp-stage.md` | 411 | new |
| this card | new | owned doc path |

No file under `crates/` or any other doc was touched.

## 6. Gates and provenance

- **No `cargo test`, no `cargo build`, no `cargo fmt/clippy`** — this
  lane adds two markdown files, which no Rust gate covers, and the
  lane constraints forbid test/build.
- Read-only inspection only (`read`, whole-tree `Select-String` for
  `fn process(` / `.process(` / `ResidentChain` / `DspStage` impls,
  `git log`), plus the one worktree-creation command in §1.
- No Winamp source, SDK, disassembly or forbidden-tree contents were
  read. No third-party code copied. Worktree-local `git status --short`
  is the final boundary receipt below; it lists exactly the two files
  above.
- `git log` base: `db461c6` (M14 review record); RFC 0012 accepted as
  `e3f73be` is in history.
