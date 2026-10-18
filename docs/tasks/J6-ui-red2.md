# Task card J6 — Green the UI crate's red tests, second pass (wave 70)

**Date** 2026-10-18. **Lane** `rustamp-ui`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave70-ui-red2`.
Write only owned paths inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.
(The only files written outside the worktree are scratch scripts and logs in
the approved temp dir `C:\Users\micro\AppData\Local\Temp\opencode`.)

**Status:** analyzed, fixed, host compilation/linting verified green, VM test
execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` were used. No window opened, no audio
device opened, no audio played, no download, no dependency addition, no
commit or other git write, no golden image touched or blessed.

## 0. Boundary

Owned paths: `crates/rustamp-ui/**` and this card `docs/tasks/J6-ui-red2.md`.
No file under `tests/golden/**`, no other crate, and no other task card was
touched.

## 1. What the VM reported, and what was established by analysis

The VM reported the `rustamp-ui` crate with **202 passed and 2 failed** — the
same numbers the J5 lane saw earlier tonight. Analysis of the test inventory
shows what those numbers are:

- The crate has six test binaries: the lib unit tests (205 `#[test]`
  attributes in `src/`, of which 1 is `#[ignore]`d → **204 runnable**) plus
  five integration binaries (`render` 14, `playlist_window` 6, `menu_render`
  6, `golden_lookalike` 15, `golden_rust_theme` 15) → 260 runnable tests in
  total.
- "202 passed and 2 failed" is exactly the **lib unit-test binary's** cargo
  summary line (`202 passed; 2 failed; 1 ignored`). The two reds are **lib
  unit tests**, not the two golden integration tests the J5 lane diagnosed.
- Both failing tests came in with `35cefe1` ("Close the playlist S06 gaps and
  give the load dialog a path", 2026-10-17 19:14), whose card
  (`docs/tasks/S06i3-playlist-visibility.md`) records its tests as *"headless,
  written, **not run** here"* — they were compiled (clippy `--all-targets`)
  but never executed until the VM ran them. The J5 lane never ran a test
  either and inferred the two reds were the unblessed `rust-theme` goldens;
  the orchestrator's bless `9600c95` did fix those two golden cases (they
  live in the `golden_rust_theme` binary), but the lib's two reds were never
  addressed. The count looked unchanged because the lib line read the same
  before and after the bless.
- The playlist window seam (`6d5f39b` + lint fix `b2878f8`) **moved nothing
  test-visible**: it changed only `playlist_key_action`'s mapping for
  `Alt+Up`/`Alt+Down`/`Ctrl+Delete`, added four `PlaylistKeyAction` variants,
  a `range_ids` helper and two write-only fields (`shift_anchor`, `current`).
  The only test of the map
  (`the_playlist_key_map_covers_the_provisional_rows_and_enter`,
  `src/playlist.rs`) does not assert any of the changed combinations and
  still passes, and **no consumer of the map exists** — `grep` over
  `crates/rustamp-app` finds no `playlist_key_action`/`PlaylistKeyAction`
  (the app's `playlist_key_down` handles plain `Delete`, `Ctrl+A`, `Ctrl+I`
  on its own path). So: the two current reds are **different tests** from the
  two the earlier lane fixed, and they pre-date the seam.

### 1.1 The two failing tests

1. **Test 1:** `handle_drag_mapping_round_trips_every_offset`
   * **Location:** `crates/rustamp-ui/src/playlist_window.rs:673` (failing
     assertion at :679; the mapping under test at :214-221 and :274-282).
   * **Verdict:** the **expectation is wrong** (it claims a mathematically
     impossible property) **and the code is wrong** (its rounding contradicts
     the code's own documentation of the inverse relationship).
   * **Reasoning:** the test asserted that every one of the 57 scroll offsets
     of a 60-entry list maps to a handle pixel y and back to itself. The
     handle travels 40 px (y = 20..=60 → **41 integer pixel rows**), so 57
     offsets share 41 rows: no mapping can be injective and a perfect
     offset-direction round trip is impossible for *any* implementation
     (pigeonhole). Arithmetic on the shipped formulas (integer division
     `40*o/56` forward, `(d*56+20)/40` back) fails for 32 of 57 offsets —
     the first failure is offset 1 → y 20 → offset 0. The code was also
     internally inconsistent: `scroll_offset_for_handle_y`'s doc claimed it
     is "the linear inverse of [`scroll_handle_y`] … so every offset
     round-trips exactly", but `scroll_handle_y` **floored** while the
     inverse **rounded to nearest**, so the pair was not inverse in either
     direction (the drag direction y → offset → y failed for 16 of 41
     positions, e.g. y 21 → offset 1 → y 20).
   * **Fix (both directions, no claim weakened):**
     * *Code* (`scroll_handle_y`, :217-228): round to the nearest row,
       `(SCROLL_TRAVEL_PX * clamped + rows / 2) / rows`, matching the
       inverse. The pair is now exact inverses **on the pixel grid**: every
       handle position y maps to an offset and back to y — verified
       arithmetically for all 41 positions (41/41 exact; with the old
       flooring 16 of 41 failed). This is the direction that governs
       dragging: a dragged handle now renders under the cursor. The three
       settled S01 §4.2 points (offsets 0/28/56 → y 20/40/60) are exact
       multiples and **unchanged** by the rounding, so no golden image and no
       pixel test that pins a settled offset is affected (the playlist golden
       cases render offsets 0, 28 and 56 only; `tests/playlist_window.rs`
       pins offsets 0 and 56).
     * *Test* (renamed `handle_drag_mapping_round_trips_every_position`,
       :684-712): the impossible loop was replaced by the satisfiable exact
       claim in the drag direction — for every y in `SCROLL_TOP_Y..=SCROLL_TOP_Y
       + SCROLL_TRAVEL_PX`, `scroll_handle_y(scroll_offset_for_handle_y(y, 60),
       60) == y` — plus the strongest true statement of the offset direction,
       `back.abs_diff(offset) <= 1` for every offset (verified: max error 1,
       exactly the offsets that share a pixel row). All original assertions
       (ends, clamps, the settled middle y = 40 → 28, the one-page list) are
       kept. Nothing was deleted to make the test pass; the impossible claim
       was corrected to the exact claim the design can and now does deliver,
       and the doc of `scroll_offset_for_handle_y` (:265-273) now states the
       pigeonhole honestly instead of the false "every offset round-trips
       exactly".
2. **Test 2:** `a_playlist_path_needs_its_dialog_and_a_path`
   * **Location:** `crates/rustamp-ui/src/dialogs.rs:1604` (failing assertion
     at :1630; the field set at :1620, the `Backspace` at :1629).
   * **Verdict:** the **expectation is wrong** (an arithmetic error); the
     **code is right**.
   * **Reasoning:** the test sets the load-playlist field to
     `"  mix.m3u  "` — whitespace-padded on purpose, so the preceding
     assertion pins that confirm trims *surrounding* whitespace
     (`Some(PathBuf::from("mix.m3u"))` at :1621-1624). It then presses
     `Backspace` once and asserted the field is `"  mix.m3"`. But
     `DialogManager::key` pops exactly one character of the raw field
     (`src/dialogs.rs:458-460`, as documented: "Backspace deletes the last
     character"), and the field's last character is one of the two trailing
     spaces — so one pop leaves `"  mix.m3u "` (one trailing space), not
     `"  mix.m3"`. The expected value forgot the trailing spaces the test
     itself had just set two lines earlier.
   * **Fix:** the expected value at :1630 is corrected to `"  mix.m3u "` with
     a comment explaining the remaining trailing space. The test's claims
     are unchanged and unweakened: `Backspace` is consumed, removes exactly
     one character, and the both-side trim-on-confirm pin stays.

### 1.2 Hygiene fix beyond the two failures

The seam's doc on `playlist_key_action` (`src/playlist.rs:148-156`) said the
`Delete`/`Ctrl+Delete` actions "can be dispatched directly via
[`PlaylistWindow::handle_key_action`]" — a method that **exists nowhere in
the workspace** (dangling intra-doc link and a false claim). The doc now
states that nothing dispatches these actions yet (the app does not consult
this map) and that applying them is app-side follow-up work.

### 1.3 Observations for the follow-up lanes (not failures)

* The seam's `shift_anchor` and `current` fields on `PlaylistWindow`
  (`src/playlist.rs:261-262`) are write-only so far (maintained in
  `set_entries`, never set), and `range_ids` (:240) has no callers. The
  gesture methods they anticipate (shift-click, ctrl-click, drag reorder,
  `Delete`/`Crop`/`MoveBlock*` dispatch — the S06i4 seam spec §3) are still
  missing; that is the S06i5 lane's follow-up, not a test failure.
* The seam's `PlaylistKeyAction::Delete` variant is currently unreachable
  through `playlist_key_action` (plain `Delete` returns `None` to preserve
  the app path, per the map's doc).

## 2. What only the VM can prove

Host execution of `cargo test` is prohibited, so only the Hyper-V VM
orchestrator can prove:

1. That the two named tests now pass and the lib binary reads
   `204 passed; 0 failed; 1 ignored` (crate total 260 passed across its six
   test binaries).
2. That the golden suite still passes with the `scroll_handle_y` rounding
   change. Statically the change cannot move a pinned pixel: the playlist
   golden cases render scroll offsets 0/28/56, which are exact multiples
   under both roundings — but only the VM's `cargo xtask golden` /
   `cargo test -p rustamp-ui` run can prove the PNGs still match
   byte-for-byte. **No golden image was blessed or modified on this host.**
3. That the full workspace suite is green after the merge.

## 3. Verification and hygiene

* `cargo fmt --all -- --check`: **PASS** (exit 0; run twice — after the edits
  and again at the end).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, zero warnings; run twice — after the edits and again at
  the end. The second run re-checked the workspace and stayed green.)
* `cargo build --workspace --offline`: **PASS** (exit 0). Note: the host's
  system drive (C:) hit **0 bytes free** during this lane — every build
  failure along the way was ENOSPC at link/codegen time (os error 112,
  `LNK1318`, `LNK1201`, rustc-LLVM "no space on device"), never a code
  error; the earlier clippy pass had already compiled every workspace
  target cleanly. Recovery deleted only regenerable caches **inside this
  worktree**: `target/debug/incremental` (pure cache, not fingerprinted),
  `.o` codegen intermediates, orphaned `deps` PDBs/exes, and one stale
  `deps/rustamp.exe`. The final build compiled and linked the whole
  workspace green (last run: only `rustamp-app` recompiled, 2.24 s).
* Constraints verified: **no `cargo test` run**, no VM run, no window opened,
  no audio device accessed, no download, no `cargo add`, no new dependency,
  no golden image touched, **no commit or other git write**.
* `git status --short` at the end shows exactly the three modified source
  files plus this card — nothing else, and nothing outside the worktree.

## 4. Files and line counts

`git diff HEAD --stat`: 3 files changed, 38 insertions(+), 11 deletions(-).

| File | Change | Lines |
|---|---|---|
| `crates/rustamp-ui/src/playlist_window.rs` | `scroll_handle_y` rounds to nearest (+doc), `scroll_offset_for_handle_y` doc corrected, test renamed and rewritten | 739 total (+30/−8) |
| `crates/rustamp-ui/src/dialogs.rs` | one expected value corrected + comment | 1653 total (+4/−1) |
| `crates/rustamp-ui/src/playlist.rs` | dangling `handle_key_action` doc reference removed, dispatch status stated | 1049 total (+4/−2) |
| `docs/tasks/J6-ui-red2.md` | this card | new |
