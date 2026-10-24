# S06i3 — Playlist visibility, load path and remaining interactions

Status: implemented in the worktree, headless tests written, **not run**
(the suite runs in the Hyper-V test VM; `cargo test` was not executed on
the host). Verified on the host by `cargo fmt --all -- --check` (clean),
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
(exit 0), `cargo build --workspace` (exit 0) and
`cargo xtask golden --skin lookalike --bless` (15 passed, 0 failed).

## Goal

Close the findings BETA-07 and BETA-09 (as far as `rustamp-ui` reaches)
and the S06 interaction gaps the S06i2 lane left as spec gaps: the
scrollbar handle drag / track click and the keyboard page keys.

## Owned paths

- `crates/rustamp-ui/src/playlist.rs` (model + key map)
- `crates/rustamp-ui/src/playlist_window.rs` (scrollbar hit-testing/mapping)
- `crates/rustamp-ui/src/dialogs.rs` (load-playlist dialog path + skip lines)
- `crates/rustamp-ui/src/lib.rs` (re-exports only)
- `crates/rustamp-ui/tests/golden_lookalike.rs` (two new cases)
- `tests/golden/lookalike/` (two new PNGs only)
- `tests/golden/README.md` (new rows only)
- `docs/tasks/S06i3-playlist-visibility.md` (this card)

Not touched: `crates/rustamp-app`, `crates/rustamp-playlist`,
`crates/rustamp-skin`, `crates/rustamp-audio`, `crates/rustamp-ipc`,
`crates/rustamp-mediahost`, `xtask`. All app-side wiring below is a
specification for the lane that owns `rustamp-app`, not an edit.

## Sources

- Specs: `specs/S06-playlist.md` (§§2-4, §8 Q3/Q5-Q7), `specs/S01-window-geometry.md`
  (§4.1/§4.2), `specs/S03-shortcuts.md` (§2 playlist table, §5 mouse gestures),
  `specs/S05-docking.md` (§1.5, §2.1).
- Cards: `docs/tasks/BETA-readiness.md` (BETA-07, BETA-09),
  `docs/tasks/S06i2-playlist-interactions.md` (gaps),
  `docs/tasks/N5-first-minute.md` (the app side that already landed),
  `docs/tasks/E3-playlist-skips.md` (the report APIs).
- No dirty-room material was read. No reference binaries were run.

---

## 1. BETA-07 — visibility: what is wired today

The finding (BETA-07) predates the N5 lane: the playlist model said closed
while the OS window was up, and its toggle/check could not hide it. That
wiring **already exists** in this worktree, put there by N5 (see
`docs/tasks/N5-first-minute.md` §3); nothing below redoes it. File:line
(the app half, owned by the other lane, read only):

| Fact | File:line |
|---|---|
| Model starts open, matching the window the presenter opens at launch | `crates/rustamp-app/src/app.rs:199` (`playlist_open: true`) |
| `App::playlist_open()` reads the UI flag; the presenter "creates or drops the OS window to match it" | `crates/rustamp-app/src/app.rs:287-289` |
| `Windowed::sync_playlist_window` creates/drops the OS window to match the flag | `crates/rustamp-app/src/window.rs:1041-1049` |
| OS window creation (275 x 116, non-resizable, same shell as main) | `crates/rustamp-app/src/window.rs:1056-1093` |
| Native close hides via `close_playlist_window` + re-sync, never quits | `crates/rustamp-app/src/window.rs:1129-1135` |
| Skinned Close hides via `ToggleWindow(Playlist)`; Shade notes S05 §1.5 | `crates/rustamp-app/src/playlist_window.rs:269-292` |
| Behaviour flips `playlist_open` on the PL toggle | `crates/rustamp-ui/src/behaviour.rs:411-413` |
| Menu check mark follows the same flag | `crates/rustamp-ui/src/menu.rs:170` |
| Resync call sites (open/close/input/redraw paths) | `crates/rustamp-app/src/window.rs:1133,1201,1262,1399,1449` |

**UI-half change in this card: none needed.** The UI machine already flips
the one flag every surface reads, and its default (`MainWindowState::default`
`playlist_open: false`, `crates/rustamp-ui/src/state.rs:146`) is correctly
overridden by the app at construction. The toggle/check/hide agreement is
pinned by the app lane's tests
(`the_title_bar_close_hides_the_editor_and_pl_reopens_it`,
`playlist_window.rs::the_title_bar_close_hides_the_editor_and_pl_reopens_it`
in `crates/rustamp-app`), which this lane may not edit.

**What the app half must still do (other lane):** nothing open remains for
visibility itself. The playlist keyboard still takes the shared path
(`window.rs::playlist_event` routes keys to `App::key_down`; see §4 for the
UI-half map waiting for it).

---

## 2. BETA-09 — a playlist file must lead somewhere

Existed: LIST OPTS opens the `Load Playlist` dialog (S06 §4) and emits
`Command::OpenPlaylist` (RFC 0010), but the dialog had **no field and no
confirmation**: `DialogManager::confirm_load_playlist` returned `None`
(`crates/rustamp-ui/src/dialogs.rs`, now line 838), the app consumed Enter
and kept the dialog open
(`crates/rustamp-app/src/app.rs:2656-2674`
`the_load_playlist_dialog_keeps_its_confirmation_open`), and no caller read
a playlist file — agreeing with E3's caller audit. E3 added the library
reporting APIs (`load_playlist_file_report`, `PlaylistReport { entries,
skips }` in `crates/rustamp-playlist`); this crate must not name those
types, so it stays free of a playlist dependency (no new dependencies were
added).

Changed (UI half, this card):

- The load-playlist dialog now carries a path the same way the open-media
  faces carry theirs: typed text through `DialogManager::type_text`
  (`dialogs.rs:540`), `Backspace` correction (`dialogs.rs:441-450` arm),
  one staged pick for a later native picker (`stage_playlist_path`,
  `dialogs.rs:650`), one dropped file (`drop_playlist_path`,
  `dialogs.rs:662`), and `confirm_load_playlist_path` (`dialogs.rs:676`)
  handing the staged pick (else the trimmed typed field) to the app.
  The dialog is included in `accepts_text` (`dialogs.rs:520`); `open` /
  `close` / `close_any` clear the new fields with the old ones.
- `confirm_load_playlist` still returns `None` (`dialogs.rs:838`): RFC 0010
  leaves replace-versus-append open, so the dialog must not silently choose
  append semantics by emitting `AddItems`. The path confirm is deliberately
  separate from the (absent) command confirm; the doc says so.
- `playlist_load_summary_lines` (`dialogs.rs:851`) formats the E3 report for
  surfacing: one count line
  (`loaded {n} track(s) ({m} skipped) from {path}`) plus one
  `skipped: {reason}` line per skip, in order, taking plain strings so the
  app can pass `SkippedEntry::reason`s without this crate depending on the
  playlist crate. Truncation stays with the app's note cap.

Tests (headless, written, not run here): `a_playlist_path_can_be_typed_staged_or_dropped`,
`a_playlist_path_needs_its_dialog_and_a_path`,
`the_load_summary_names_counts_and_every_skip` (all in `dialogs.rs`), plus
the updated `the_jump_box_takes_typed_digits_and_colons_only`, which no
longer asserts the dialog is field-less.

**What the app half must do (other lane, `crates/rustamp-app`):**

1. On `Enter` with `DialogKind::LoadPlaylist` open, call
   `confirm_load_playlist_path`; with `Some(path)`, call
   `rustamp_playlist::load_playlist_file_report(&path)` (E3 §4.2), append
   the surviving entries with `append_loaded` semantics, close the dialog
   (the one new close this half enables), and surface
   `playlist_load_summary_lines(&path, report.len(), &reasons)` through
   notes/stderr; with `None`, keep the dialog open as today.
2. Decide replace-versus-append per RFC 0010 and record the choice in a
   follow-up card; until then the card's test
   (`the_load_playlist_dialog_keeps_its_confirmation_open`) must be updated
   to the new behaviour.
3. Where the report surfaces: the same two channels N5 used for open
   errors — stderr plus the native title bar (`App::last_error`) — since
   the built-in placeholder font cannot be relied on for in-skin text
   (BETA-02) and the dialog closes on confirm.

---

## 3. Scrollbar handle drag and track click

Existed: arrows hit-test (`hit_test_scroll`, `playlist_window.rs:227`) and
one-page steps (`scroll_page`, `playlist.rs`), handle drawn at
`scroll_handle_y` (`playlist_window.rs:214`). S06 §3 settles only that a
dragged handle "moves the view" (a visual reading toward the top); **no spec
fixes the pixel-to-row mapping**, and track clicks are unspecified entirely.
`playlist_mouse_down` does not route handle/track clicks (app-owned; see the
app-half note).

Changed (UI half, all marked own choices in the docs):

- `SCROLL_TRACK_RECT` (`playlist_window.rs:246`): the 8 px column from the
  top settled point (y = 20) down to the arrow pair (y = 78).
- `scroll_handle_rect` (`playlist_window.rs:250`): the 7 x 17 handle rect
  for an offset.
- `scroll_offset_for_handle_y` (`playlist_window.rs:262`): the linear
  inverse of `scroll_handle_y` (itself an own-choice interpolation),
  clamped to the track ends, rounded to the nearest row, so every offset
  round-trips exactly.
- `ScrollbarHit` + `hit_test_scrollbar` (`playlist_window.rs:281,292`):
  Handle (drag it), TrackAbove (page up), TrackBelow (page down) — the
  track behaviour is an own choice (usual track paging); the arrow pair
  keeps its own path and returns `None` here.

Tests (headless, written, not run here):
`handle_drag_mapping_round_trips_every_offset` (all 57 offsets round-trip,
ends, clamps, the settled y = 40 → 28 middle) and
`scrollbar_hit_testing_separates_handle_track_and_arrows`.

**What the app half must do (other lane):** route handle/track clicks in
`App::playlist_mouse_down` — a press on `ScrollbarHit::Handle` starts a drag
(the presenter feeds motion y values into `scroll_offset_for_handle_y` +
`scroll_to`), `TrackAbove`/`TrackBelow` call `scroll_page(Up/Down)`.
`crates/rustamp-app/src/playlist_window.rs` is the place; this card does not
touch it.

---

## 4. Keyboard page keys

Existed: `App::playlist_key_down` handles Delete, Ctrl+A and Ctrl+I and
passes everything else to the shared `App::key_down`
(`crates/rustamp-app/src/playlist_window.rs:156-179`); S06i2 left the S03 §2
arrow/Home/End/page rows there because S03 marks them **provisional** and
leaves how the tables combine provisional too.

Changed (UI half, all provisional rows implemented as own choices and
marked so):

- `PlaylistKeyAction` + `playlist_key_action` (`playlist.rs:124,151`):
  plain Up/Down/Home/End/PageUp/PageDown/Enter map to
  `MoveSelectionUp/Down`, `JumpToFirst/Last`, `PageUp/Down`, `PlaySelected`;
  everything with modifiers — and Delete/Ctrl+A/Ctrl+I, which keep their
  existing app path — returns `None` and stays on the shared path.
- Model methods (`playlist.rs:382-448`): `move_selection_up/down`
  (collapse to a single selection one entry above/below the playlist-order
  anchor; with no selection, the last/first visible row; the view follows),
  `jump_to_first/last` (select the end entry alone, reveal it),
  `play_selected` (plays the **first** selected entry in playlist order and
  selects it alone — the spec fixes the binding `40184` but not which entry
  plays when several are selected, so this is an own choice; `None` when
  empty). PageUp/PageDown reuse the existing `scroll_page` (S03 says
  "scroll", so the selection is kept — own choice).

Tests (headless, written, not run here): `up_and_down_move_the_selection_and_reveal_it`,
`home_and_end_jump_to_the_ends_of_the_list`,
`enter_plays_the_first_selected_entry`,
`the_playlist_key_map_covers_the_provisional_rows_and_enter`.

**What the app half must do (other lane):** extend
`App::playlist_key_down` to consult `playlist_key_action` first: apply
move/jump methods directly (view-only, then submit nothing), `scroll_page`
for page keys (view-only), and `submit` the command `play_selected`
returns. `crates/rustamp-app/src/window.rs::playlist_event` still routes
keys globally; pointing it at `playlist_key_down` is app-lane work already
noted in S06i2 §4.

---

## Own choices (complete list)

1. Linear handle interpolation inverse with round-to-nearest (§3).
2. Track column rect `(260,20,8,58)`; track-above = page up, track-below =
   page down (§3).
3. Cursor-less selection movement collapsing to one entry; empty-selection
   anchors (Down → first visible, Up → last visible); view-follows rule
   (§4).
4. Home/End select the end entry alone (§4).
5. Page keys scroll and keep the selection (§4).
6. Enter plays the first selected entry in playlist order (§4).
7. Load-path summary line wording; truncation left to the app's note cap
   (§2).
8. New goldens' labels and typed-path body (renderer chrome is S04-open).

## Goldens (`lookalike`, `cargo xtask golden --skin lookalike --bless`)

| Case | Dimensions | What it pins |
|---|---|---|
| `playlist_window_mid_page` (new) | 275 x 116 | Offset 28 of 60 (handle at the settled y = 40) with row 30 selected via `move_selection_down`; own-choice interpolation + provisional Down path |
| `dialog_load_playlist` (new) | 300 x 100 | Load-playlist dialog with typed `C:\Music\mix.m3u`; the BETA-09 typed face |

Re-blessed only the two new images (`tests/golden/lookalike/playlist_window_mid_page.png`,
`tests/golden/lookalike/dialog_load_playlist.png`);
`playlist_window.png` and `playlist_window_selection.png` were re-blessed
byte-identical (no diff in `git status`). Golden run: 15 passed, 0 failed
(the 13 pre-existing lookalike cases plus the 2 new ones).

## Files and line counts (`git diff --stat` + 2 new PNGs)

| File | Change |
|---|---|
| `crates/rustamp-ui/src/dialogs.rs` | +path field, confirm, skip lines, tests |
| `crates/rustamp-ui/src/playlist.rs` | +key map, selection methods, tests |
| `crates/rustamp-ui/src/playlist_window.rs` | +track/drag mapping, hit-testing, tests |
| `crates/rustamp-ui/src/lib.rs` | re-exports only |
| `crates/rustamp-ui/tests/golden_lookalike.rs` | 2 new cases |
| `tests/golden/README.md` | new rows |
| `tests/golden/lookalike/playlist_window_mid_page.png` | new, 275 x 116 |
| `tests/golden/lookalike/dialog_load_playlist.png` | new, 300 x 100 |
| `docs/tasks/S06i3-playlist-visibility.md` | this card |

`git diff --stat`: 6 files, 674 insertions, 31 deletions.

## Verification and boundary receipt

- `cargo fmt --all -- --check`: clean (after one `cargo fmt --all` pass).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo build --workspace`: exit 0.
- `cargo xtask golden --skin lookalike --bless`: 15 passed, 0 failed; no
  mismatch artefacts under `target/golden-out`.
- `cargo test` was **not** run (suite runs in the Hyper-V test VM); the new
  tests were compiled (`clippy --all-targets`) but **not executed**. The
  golden run above executes only `golden_*` tests headlessly, which is the
  allowed exception — no window opened, no audio played, no download, no
  commit, no `cargo add`, no new dependency.
- `git status --short` (inside the worktree) shows only the files above:
  6 modified + 2 new PNGs; nothing under `D:\code\Rustamp`,
  `D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or
  written (those trees were never inspected — not even to read).
