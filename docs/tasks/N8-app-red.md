# Task card N8 — Green the app crate and wire easy-move (wave 65)

**Goal**
Resolve the two red tests the VM reported in `rustamp-app` (140 passed,
2 failed), wire the one setting the batch review proved is advertised but
unhonoured (`ui.easy_move`, W10 finding F5), and re-check the
BETA-readiness-2 claim that Load-Playlist Enter "is consumed and does
nothing" now that N7 has landed.

**Lane** N — application.

**Owned paths**
- `crates/rustamp-app/src/**`
- `crates/rustamp-app/tests/**`
- `docs/tasks/N8-app-red.md`

Do **not** touch `rustamp-ui`, `rustamp-platform`, `rustamp-core`,
`rustamp-playlist` or any other crate — other lanes own them right now.
In particular the fix for failing test 1 below touches only the app-side
*expectation*; the contract strings it asserts live in `rustamp-playlist`
and were read, not changed. The fix for failing test 2 touches only the
app-side *writer*; the loader and the `WindowLayout` type are untouched.

**Allowed sources**
- `specs/S03-shortcuts.md` §1 (Ctrl+E / 40186 "Toggle easy move", menu
  confirmation) and §5.
- `specs/S01-window-geometry.md` §2.1/§2.2 via `MainWindowLayout::CLASSIC`
  (the background point used by the new test).
- `specs/S06-playlist-window.md` §4 (the "Load Playlist" title).
- `rustamp-core/src/config.rs:126-127` (the contract gloss of the setting:
  "Drag windows from anywhere").
- `docs/tasks/W10-batch-review.md` (finding F5), `docs/tasks/N6-config.md`,
  `docs/tasks/N7-playlist-load.md`, `docs/tasks/BETA-readiness-2.md`
  (§1 BETA-09, §3 item 3).
- Clean-room Rustamp repository code (read-only where not owned).

**Forbidden sources**
- Winamp source code, SDK headers, disassembly, dirty room
  (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`, `D:\code\Rustamp`
  as a write target — the lane worked in its own worktree only).

**Worktree note.** The assigned path
`...\26de0e\wave65-app-red` did not exist (the previous Google job failed
before writing, and no worktree was ever registered for this lane). To do
the work without touching the canonical tree, the lane materialised that
directory as a pristine filesystem copy of the canonical `main`
(`2ebaaf0`, verified clean with `git status --short` in `D:\code\Rustamp`
before copying), excluding `target/` and `.git/`. It is therefore not a
git worktree: `git status --short` inside it reports "not a git
repository" rather than modifications, and the starting state is exactly
the canonical `main` content. No file outside the assigned directory was
read except read-only sources, and none was written.

**Scope**
- **In scope:**
  1. Establishing, for each of the two red tests, what it asserts and why
     it fails — code wrong or expectation wrong — with `file:line`, and
     fixing the cause without weakening what the test claims.
  2. Wiring easy-move into a real window drag, or removing the setting
     honestly if wiring is not coherent (see §3 for the choice).
  3. Re-checking BETA-readiness-2's Load-Playlist Enter claim against the
     N7 implementation now in the tree.
  4. Headless tests for the new drag decision.
- **Out of scope:**
  - Modifying `rustamp-core` contracts (RFC required), the playlist
    reader's reason strings, the UI hit-test tables, or platform code.
  - Editing other lanes' cards (`N6`, `N7`, `BETA-readiness-2`): §5 lists
    the lines that are now stale instead.

## 1. Failing test 1 — the skip reason was guessed, not derived

**Test:** `playlist_file_with_bad_entry_loads_surviving_and_reports_skips`
(`crates/rustamp-app/src/app.rs:3580`, added by N7, never executed).

**What it asserts:** a playlist with a dangling `#EXTINF` loads its two
surviving entries in order, starts playback of track 0, records one skip,
and names the reason in the report and in the notes.

**Why it fails:** the expectation was transcribed from the N7 card prose
("dangling EXTINF") instead of derived from the reader contract. The
reader's contract string is `"dangling #EXTINF without a location line:
<title>"` (`crates/rustamp-playlist/src/m3u.rs:80-81,119-120`), so the
first failing assert (`app.rs:3621`,
`report.skips[0].reason().contains("dangling EXTINF")`) is false — there
is a `#` between "dangling" and "EXTINF". The second substring assert
(`app.rs:3631`, `"skipped: dangling EXTINF"`) fails for the same reason.
**Verdict: the expectation was wrong, the code was right.** The queue,
order, playback, skip count and summary-note asserts all trace clean
against the implementation.

**Fix (this lane):** both asserts now name the contract string
`"dangling #EXTINF"` (`app.rs:3621,3631`). This does not weaken the
claim — the test still pins exactly one skip with the reader's exact
reason, in the report and in the surfaced note.

## 2. Failing test 2 — the writer dropped the sizes it must round-trip

**Test:** `a_generic_window_id_survives_the_written_file`
(`crates/rustamp-app/src/config.rs:442`, added by the F1 repair, never
executed).

**What it asserts:** a config holding a `WindowId::Gen(3)` window with
`width: Some(275), height: Some(232)` saves and loads back equal.

**Why it fails:** `serialize_honest` (`config.rs:291-312`) writes
`id/x/y/visible/shaded` but never `width`/`height`, while the loader
reads them through serde (`toml::from_str` into `Config`; serde defaults
absent `Option` fields to `None`). The loaded window therefore comes back
with `width: None, height: None` and the equality assert (`config.rs:473`)
fails. **Verdict: the code was wrong, the expectation was right.** It is
the same writer/loader drift as W10 finding F1, one field family over:
the type carries the sizes (`rustamp-core/src/config.rs:110-113`), the
loader accepts them, the writer discarded them.

**Fix (this lane):** the writer now emits `width`/`height` when they are
`Some` and keeps omitting them when `None` (`config.rs:313-318`), so the
three standard windows (always `None` in practice, cf. `app.rs:395-434`)
serialize byte-identically to before, while a real size survives the
round trip. No new key is advertised for windows that have no size.

## 3. Easy-move: wired, not removed

**Choice: wire it** — a main-window background press now starts an OS
window drag when the setting is on.

**Why not removal:** removal is incoherent here. The core contract names
the behaviour ("Drag windows from anywhere", `UiConfig::easy_move`,
`rustamp-core/src/config.rs:126-127`); the audited S03 keeps the Ctrl+E
menu toggle; and the N6 honest-surface rule only permits omitting
*unhonoured* settings — deleting the menu row would contradict an audited
spec, and deleting the core field would need an RFC (out of scope). With
the wiring, the persisted setting, the menu check and the behaviour
agree, and W10 finding F5 is closed.

**What the spec says:** S03 settles only the toggle (Ctrl+E / 40186,
menu-confirmed). No spec settles the drag interaction itself (S05
docking/undocking open, S04 menus open), so the interaction shape is an
own choice, stated plainly: **main window only, background presses only.**
`App::easy_drag_main(x, y)` (`app.rs:558-560`) returns true exactly when
easy-move is on and `hit_test` finds no widget — sliders, the marquee
hand-scroll, buttons, the title-bar held state and the inert command-less
widgets all keep their presses untouched. `Windowed::try_easy_drag`
(`window.rs:1283-1293`) turns that decision into `winit`'s `drag_window()`
on the main-window left-press path (`window.rs:1460-1468`); a failed drag
falls back to the control path so a press is never swallowed. The release
path is unchanged (a dragged press holds nothing, so `mouse_up` emits
nothing). EQ and playlist windows are deliberately excluded: the
implemented S05 model force-docks both followers to the main window
(`place_eq_below_main`, `sync_playlist_position`), and independent
follower dragging is exactly the open S05 §7.8 undocking question — wiring
it now would fight the dock. Dragging the main window drags the stack
through the existing `Moved` handling (S05 §4.2).

**Headless pinning:** `easy_move_decides_which_presses_drag_the_window`
(`app.rs`, next to the dialog tests) covers background-drags, a control
keeping its press, and the toggle turning all dragging off. The
background point (5, 100) was verified against every `CLASSIC` rect in
`rustamp-ui/src/layout.rs:133-177`: left of the transport row, below the
clutter bar — `hit_test` returns `None` there by construction, and the
test additionally asserts the control machine ignores the press
independently of the helper.

## 4. BETA-readiness-2 BETA-09 re-checked: no longer true after N7

The review's claim — Enter on the Load Playlist dialog "is consumed and
does nothing" (`dialogs.rs:130-133`, "RFC 0010 leaves its confirmation
open") — predates the N7 lane and is **stale**. In this tree Enter
confirms through `confirm_load_playlist` (`dialogs.rs:188-191,224-239`):
the staged/typed path is loaded via `App::load_playlist_file`
(`app.rs:1431-1491`, queue replaced, track 0 played, report recorded,
skips noted), the dialog closes; an empty confirm keeps the dialog open
(pinned by `the_load_playlist_dialog_keeps_its_confirmation_open`,
`app.rs:2974`). The dialog body shows the staged/typed tail plus the hint
(`dialogs.rs:426-431`). What remains open is the VM/user proof, not the
wiring.

## 5. What only the VM can prove

- The two fixed tests (no test was executed in this lane, per constraint;
  the suite runs in the Hyper-V VM only): the bad-entry skip assertions
  and the generic-window round trip, plus the new easy-drag decision
  test (headless, but likewise unexecuted here).
- The `drag_window()` behaviour itself: press-to-drag on the skinned
  background with easy-move on/off, no interference with sliders,
  marquee scroll, buttons or dialogs, and the docked EQ/playlist
  following the main window through the move. No window may open on the
  host, so this is VM-only by rule.
- Everything §3 of BETA-readiness-2 already marked VM-only; unchanged.

## 6. Verification receipt

- `cargo fmt --all -- --check`: exit 0 (one line re-wrapped by `cargo fmt`).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0, no warnings (this also compiles all test targets).
- `cargo build --workspace`: exit 0.
- `cargo test`: **not run** (lane constraint; the suite runs in the VM).
- No window opened, no audio played, no download, no `cargo add`, no new
  dependency, no commit, no git write. Files changed, all inside the
  assigned worktree: `crates/rustamp-app/src/app.rs` (+40, 2 lines
  corrected), `crates/rustamp-app/src/config.rs` (+9),
  `crates/rustamp-app/src/window.rs` (+23 net), plus this card (new).

## 7. Provenance

- Implemented only from `specs/` (S01, S03, S06) and clean-room Rustamp
  code. No Winamp source, SDK headers, disassembly, `D:\code\Rustamp`
  writes, `D:\code\rustamp-research` or `D:\code\rustamp-refbin` were
  involved; the canonical tree was read read-only to establish the base
  commit and the pre-existing failure evidence.
- Third-party code copied: none. License impact: none (no dependency
  change; `rustamp-app` is GPL-3.0-or-later, untouched).

## 8. Suggested follow-ups (not owned here, not edited)

- `docs/tasks/N6-config.md:36` still records easy-move as "not honoured
  in drag handling" — stale after §3.
- `docs/tasks/BETA-readiness-2.md` §1 BETA-09 and §3 item 3 still record
  the dead Enter — superseded by N7 + §4.
- The VM run should name the app-crate results explicitly so the next
  greening lane does not have to re-derive them by reading.
