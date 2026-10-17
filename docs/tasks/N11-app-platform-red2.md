# Task card N11 — Green the app and platform crates, second round (wave 70)

**Goal**
Fix the three red tests the VM reported (`rustamp-app`: 146 passed,
2 failed; `rustamp-platform`: 57 passed, 1 failed): for each, decide
whether the code is wrong or the expectation is, with `file:line` and
the reasoning, then fix the cause without weakening what the test
claims. Make the platform failure independent of the machine it runs
on, so the suite passes on a guest with one display.

**Lane** N/F — greening (`rustamp-app`, `rustamp-platform`).

**Owned paths**
- `crates/rustamp-platform/**`
- `crates/rustamp-app/**`
- this card, `docs/tasks/N11-app-platform-red2.md`

Do **not** touch `rustamp-ui` or any other crate — other lanes own them
right now. No window, no audio, no downloads, no `cargo add`, no new
dependencies, no git writes. `cargo test` was **not** run here (the
suite runs in the Hyper-V VM only); only `cargo fmt --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo build --workspace` were run on this host.

**Allowed sources**
- `docs/tasks/N8-app-red.md` (the previous app greening: skip-reason
  expectation fix, width/height writer fix, easy-move wiring).
- `docs/tasks/N9-config-class.md` (serde delegation for `WindowLayout` /
  `EqState`, exhaustive round-trip tests).
- `docs/tasks/F4-platform-displays.md` (display module, own-choice clamp
  rule, fake-enumeration tests).
- `docs/tasks/BETA-readiness-2.md` (§2h: the three coexisting EQ
  defaults).
- Clean-room Rustamp repository code (read-only where not owned) and
  the vendored `toml` sources in the local Cargo registry (to verify
  `None` handling, not copied code).

**Forbidden sources**
- Winamp source, SDK headers, disassembly, dirty room
  (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`).
- Nothing was read or written outside the assigned worktree
  (`...\26de0e\wave70-app-platform2`); in particular `D:\code\Rustamp`
  was never touched. Provenance: `git status --short` at the end shows
  only the files below.

## 1. Platform failure — the test data was never orphaned (expectation wrong, code right)

**Test:**
`a_saved_position_off_every_monitor_lands_wholly_inside_the_target`
(`crates/rustamp-platform/src/displays.rs:641`).

**What it asserts:** four saved positions, each supposedly off every
monitor, clamp wholly inside the primary work area.

**Why it fails:** two of the four positions are not off every monitor
at all. The fake set is primary `(0,0,1920,1080)` with work area
`(0,0,1920,1032)` plus secondary `(1920,0,1920,1080)`. A 275x116
window at `(-100,-100)` spans `x -100..175`, `y -100..16`, which
intersects the primary work area over a 175x16 strip; `(-1,-1)` spans
`-1..274`, `-1..115` and likewise intersects. `clamp_position`
(`displays.rs:227-253`) therefore returns both positions unchanged, by
its minimal-intervention rule (a window that still touches a work area
is left exactly where it was) — and the test's wholly-inside asserts
(`rect.x >= 0`, `rect.y >= 0`) fail on the unchanged negative
coordinates. This fails deterministically on **every** machine; it is
not host-dependent and not about the guest's display count.

**Verdict: the expectation (test data) was wrong, the code was right.**
The clamp logic, the minimal-intervention rule and the wholly-inside
seating are exactly the behaviour F4 pins elsewhere (both far-corner
positions clamp correctly and are covered by the sibling test at
`displays.rs:574`).

**Fix (this lane):** the two non-orphaned positions are replaced with
genuinely orphaned ones, keeping the test's four-corner shape —
`(-100,-100)` becomes `(-1000,-1000)` and `(-1,-1)` becomes
`(-2000,5000)` (`displays.rs:646-651`). Each replacement misses both
work areas (negative x-range), clamps to `(0,0)` resp. `(0,916)`,
and lands wholly inside the primary work area. The claim is
strengthened, not weakened: the test now actually exercises orphaned
positions on all four sides instead of asserting something false about
two visible ones.

**Machine-independence note (stated plainly):** the orchestrator's
guess that this test "assumes something about the host's own
monitors" is wrong. The test uses only fake snapshots; the live
`capture()` test (`displays.rs:848-892`) already carries the session-0
carve-out and passes with zero, one or many monitors. No
machine-dependent assumption was found or removed — the data was
simply not what the test claimed it was.

## 2. App failure 1 — the snapshot test still expects the disengaged default (expectation wrong, code right)

**Test:** `the_assembled_snapshot_merges_engine_and_app_state`
(`crates/rustamp-app/src/host.rs:630-658`, failing assert at `:652`).

**What it asserts:** a headless app built from `Config::default()`
publishes a host snapshot whose `eq` equals `EqState::default()`
(the wire contract's flat, **disengaged** default).

**Why it fails:** the fresh configuration has started **engaged and
flat** since `cdead5f`, deliberately (spec-open per S07 §12 item 13;
documented in `BETA-readiness-2.md` §2h and re-affirmed by N9's audit
table, which lists `EqConfig` as persisted/honoured and uses
`enabled = false` as the *non-default* probe value). `App::new`
seeds the engine from that config
(`crates/rustamp-app/src/app.rs:201`,
`engine.set_eq(options.config.eq.state)`), so `engine.eq().enabled`
is `true` and `host_player_state()` (`app.rs:839-848`, which reports
`engine.state().eq`) publishes the engaged flat state. The assert
compares it against the disengaged wire default and fails on
`enabled`. The expectation dates from lane M3d (`90c8991`) and the
"flat, disabled default" comment from the S07 wiring (`d39ce3f`) —
both predate the engaged-default decision, so **the expectation
predates the lanes that just landed, exactly as suspected**.

**Verdict: the expectation was stale, the code was right.** Reporting
the engaged flat state the engine was seeded with is the honest
behaviour the BETA review documents; the UI machine default, the
blessed renders and the config default all agree on engaged.

**Fix (this lane):** expect the fresh config's state instead of the
wire default — `assert_eq!(snapshot.eq, Config::default().eq.state)`
(`host.rs:649-652`; `Config` was already imported). The test still
pins the *entire* `EqState` for exact equality, only against the
correct default. The stale module-doc sentence is corrected with it:
"An untouched player still publishes the fresh configuration's flat,
engaged default" (`host.rs:78-79`). The doc's second half (flat gains
render as classic 31) is untouched and still true.

## 3. App failure 2 — not isolated by static inspection; audit trail and recommendation

The remaining reported app failure could not be isolated to a second
deterministic defect by reading. What was done:

- Enumerated the suite precisely: 152 `#[test]`s in
  `crates/rustamp-app/src`, 4 `#[ignore]`d
  (`app.rs:3442`, `playlist_window.rs:660`, `window.rs:1765`,
  `supervise.rs:1264`) → **148 run = 146 passed + 2 failed**, matching
  the VM report exactly (so both failures are lib unit tests, not
  integration tests).
- Re-traced every test in the N8/N9 footprint and found each
  consistent: the `dangling #EXTINF` asserts (`app.rs:3660,3672`)
  match the reader contract
  (`crates/rustamp-playlist/src/m3u.rs:80-81,119-120`); the
  width/height round trip and both exhaustive round-trip tests hold
  (verified against the vendored `toml 0.9.12` sources that struct
  fields serializing to `None` are skipped, and that missing `Option`
  fields read back as `None`); the easy-drag decision point
  (`app.rs:558-560`) was re-checked against every `CLASSIC` rect in
  `rustamp-ui/src/layout.rs:133-177` plus `hit_test`
  (`rustamp-ui/src/widget.rs:193-198`) — `(5,100)` is background by
  construction.
- Re-traced all dialog/chain/menu/playlist/EQ/supervise/host/cli/
  routing/skin/window unit tests for stale expectations and
  machine dependence (env vars, CWD, time, threads, displays, audio
  devices, locales): the only stale expectation found is §2 above,
  and no machine-dependent assertion was found (the lib suite uses
  per-test tempdirs, explicit `Instant`s and fake decoders/clocks).
- Ruled out neighbours: N9's serde switch is byte-identical in effect
  (same `try_from`, same keys); the UI playlist seam (`6d5f39b`)
  adds fields and key mappings on paths the app never routes through
  (`App::playlist_key_down` handles Delete/Ctrl+A/Ctrl+I itself);
  post-N8 audio/DSP lanes touch samples/transports the app tests do
  not pin.

**Recommendation:** the VM should name the second failure with its
`[FAIL]` line (as it did for the audio/DSP lanes). The prime
candidates if it reproduces deterministically are timing-shaped
tests, but the audit found each of them carefully guarded
(iteration-bounded pumps, parked-writer handover with filler,
explicit `poll_at` instants). If the second failure does *not*
reproduce on a re-run, it was environmental; either way no code was
changed on a guess — weakening a claim to chase an unnamed failure
would be worse than reporting it.

## 4. Verification receipt

- `cargo fmt --all -- --check`: exit 0 (after the edits).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0, no warnings (this also compiles all test targets).
- `cargo build --workspace` (offline): exit 0.
- `cargo test`: **not run** (lane constraint; the suite runs in the
  VM). No window opened, no audio played, no download, no `cargo
  add`, no new dependency, no commit, no git write.
- Files changed, all inside the assigned worktree:
  - `crates/rustamp-platform/src/displays.rs` (+5, -1: two
    genuinely-orphaned positions plus an explanatory comment).
  - `crates/rustamp-app/src/host.rs` (+4, -3: corrected snapshot
    expectation, corrected stale module-doc sentence).
  - `docs/tasks/N11-app-platform-red2.md` (this card, new).
- No other crate touched (in particular not `rustamp-ui`).

## 5. Provenance

- Implemented only from `specs/`-adjacent in-tree contracts
  (S05 §6 via F1/F4, S01 geometry, S03 §1) and clean-room Rustamp
  code plus the vendored `toml` sources (API facts for `None`
  handling, not copied code). No Winamp source, SDK headers,
  disassembly, `D:\code\rustamp-research` or `D:\code\rustamp-refbin`
  were involved; nothing outside the worktree was read except the
  task cards, and nothing outside it was written.
- Third-party code copied: none. License impact: none (no dependency
  change; `rustamp-app`/`rustamp-platform` are GPL-3.0-or-later,
  untouched).

## 6. Suggested follow-ups (not owned here, not edited)

- The VM run should name failing tests (`[FAIL]` lines) the way it
  did for the audio/DSP lanes, so the next greening lane does not
  have to re-derive them by reading.
- `docs/tasks/W14-wave-review.md` / `docs/notes/lane-jobs.md` still
  record the platform failure as "(machine-dependent?)" — §1 above
  refutes that; the test is machine-independent and was failing
  everywhere.
