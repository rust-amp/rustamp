# S06i4 — Playlist model interactions (range select, ctrl-click, drag reorder, delete)

Status: implemented in the worktree, headless tests written, **not run**
(the suite runs in the Hyper-V test VM; `cargo test` was not executed on
the host). Verified on the host by `cargo fmt --all -- --check` (clean),
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
(exit 0) and `cargo build --workspace --offline` (exit 0).

## Goal

Give the playlist **model** the editor interactions spec S06 describes but
`crates/rustamp-playlist` did not have: a contiguous range selection
(shift-click), a per-row toggle (ctrl-click), a block reorder (drag /
`Alt+Up`/`Alt+Down`), and the selection removals (`Delete`,
`Ctrl+Delete`) — with an explicit rule for what each does to the playing
entry and its index. The window half (hit-testing, pixels, dialogs) stays
with `rustamp-ui`/`rustamp-app`; this card only names the seam.

## Owned paths

- `crates/rustamp-playlist/src/interact.rs` (new module + tests)
- `crates/rustamp-playlist/src/lib.rs` (module declaration, re-exports, doc)
- `docs/tasks/S06i4-playlist-interactions.md` (this card)

Not touched: `crates/rustamp-ui`, `crates/rustamp-app`, `crates/rustamp-core`,
`crates/rustamp-meta` and every other crate. No contract change, so no RFC:
everything is built from the existing core `Playlist` API and the existing
`Command::Select` / `Command::Remove` / `Command::MoveEntries` wire types.

## Continuation note (two dead lanes)

The worktree held partial work from earlier lanes that died before reporting:
`interact.rs` (447 lines, module docs + nine functions + eight tests) and the
lib.rs wiring, both uncommitted on top of `d02dc23`. It was coherent, matched
the spec analysis, and compiled cleanly against the frozen core contract, so
this lane **continued it** rather than restarting (the HANDOFF's standing rule:
review and continue uncommitted lane work). What this lane added on top:

- `click_select` — the settled single-click selection set (S06 §2) was not
  expressible through the module before; the plain-click gesture had no model
  entry point at all.
- Fixed the module docs' citation: the dead lane attributed the settled
  "Remove the selected item(s)" to S03 §2 `1034`; the settled REM measurement
  is S06 §5 (S03 `1034` is the menu-confirmed key row). Both are now cited
  correctly and separately.
- Current-entry assertions in the `Alt+Up`/`Alt+Down` test (identity *and*
  index after each one-row move), and click coverage in the empty/single-entry
  test.
- Renamed the test helper `ids` to `entry_ids`: the local binding `let ids =
  ids(&p)` shadowed the helper function, which broke compiling the test target
  (found by `clippy --all-targets`; the dead lanes never got a green check).

## Sources

- Specs: `specs/S06-playlist.md` (§2, §5 incl. §5.2/§5.3/§5.4, §8 Q6/Q11/Q14),
  `specs/S03-shortcuts.md` (§2 playlist table rows `1034`/`1035`/`40215`/
  `40216`, §5 mouse gestures).
- Cards: `docs/tasks/E3-playlist-skips.md`, `docs/tasks/S06i3-playlist-visibility.md`,
  `docs/tasks/S06i2-playlist-interactions.md` (read-only context).
- Core contract (frozen): `rustamp_core::playlist::Playlist` — stable never-
  reused `EntryId`s, `select_only`, `toggle_selected`, `move_entries`,
  `remove`, `current`/`current_index`; `rustamp_proto::command::{Command,
  SelectOp}`.
- No dirty-room material was read. No reference binaries were run. No new
  dependencies.

---

## 1. What the spec settles, and what is an own choice

| Behaviour | Grade | Basis |
|---|---|---|
| A plain click selects exactly one row (replacing the selection) | **settled** (selection set restated) | S06 §2: single click selects the first visible row, 2918 px, reproduced three times. The pixel point is settled as "first visible row"; the row→index mapping is the window's hit-test. `click_select` implements the settled selection-set effect. |
| Shift-click extends/selects a contiguous range, replacing the previous selection | **own choice** | Not settled: S06 §8 records that beyond the all-select of §2 "nothing about SEL visuals or multi-selection is settled" (Q11, Q14 remainder). Chosen because the two settled gestures are one row and the whole list, and a selection that could be neither extended nor trimmed would make the settled removal (S06 §5 / S03 `1034`) unusable past one row. Ordinary desktop meaning: anchor..target inclusive, argument order irrelevant, both ends clamped to the last row. |
| Ctrl-click toggles one row in/out of the selection, keeps the rest | **own choice** | Same open question; ordinary desktop meaning on top of the core `toggle_selected`. |
| Drag reorders entries as a block, keeping relative order | **provisional → own choice** | S03 §5 lists "Drag in the playlist → reorder / move entries" as provisional; S06 §8 Q6 could not decide "reorder vs selection block" from the captures (activation confound). Moving the whole **selected block** matches both provisional readings at once and the core `move_entries` convention (`to` is the index in the list *without* the moved entries — the same convention as `Command::MoveEntries`). |
| `Alt+Up`/`Alt+Down` move the selected block one row; refused at the ends; a non-contiguous selection moves as one block | **own choice** on provisional rows | S03 §2 `40215`/`40216`, "Move the selected entry up/down", confirmation **provisional**, no measured effect. Keyboard face of the same block move. |
| `Delete` removes the selection; unstaged delete is a no-op | **spec-supported** | S03 §2 `1034` (menu-confirmed action text) + S06 §5 (removal settled on both paths, 3475 px) + S06 §5.3 (without a staged selection the button is a no-op, 0 px). |
| `Ctrl+Delete` crops to the selection | **spec-supported** (existence) | S03 §2 `1035`, "Crop to the selected item(s)", menu-confirmed. The unstaged-crop-is-a-no-op rule mirrors S06 §5.3 and is an **own choice** (the spec says nothing about an unstaged crop). |
| Playing entry survives a reorder and a delete-of-something-else; deleting the current entry clears `current`; a surviving entry's index shifts down by exactly the number of removed entries before it | **core contract, restated as the governing rule** | The spec does not settle what happens to the current index on these gestures (S06 §8 leaves playback-to-end and keyboard effects open). This module therefore does **not invent** a rule: it follows the frozen core `Playlist` guarantees — ids are stable and never reused, `remove` clears `current` only when the current entry is removed, indices derive from `index_of`. Every test pins identity (`current() == Some(id)`) *and* position (`current_index()`). |
| Range selection replaces (does not extend) the previous selection; the caller owns the anchor and must re-derive it with `index_of` after a reorder | **own choice** | The spec has no anchor semantics at all; the simplest rule that composes with the settled single click is stated explicitly in the function docs. |

## 2. The API (all in `crates/rustamp-playlist/src/interact.rs`)

Plain functions over `&mut Playlist` — no new types, no state, no window:

| Function | Gesture | Returns |
|---|---|---|
| `click_select(&mut Playlist, index) -> bool` | plain click | `false` past the end / on an empty list |
| `select_range(&mut Playlist, anchor, target) -> bool` | shift-click | `false` on an empty list; selection becomes the clamped inclusive range |
| `range_ids(&[EntryId], anchor, target) -> Vec<EntryId>` | same range over a bare id list | identical clamping, so window machine and model can never disagree |
| `toggle_select_at(&mut Playlist, index) -> bool` | ctrl-click | `false` past the end |
| `move_selected(&mut Playlist, to) -> bool` | drag drop | `false` when nothing is selected or the order does not change |
| `move_selected_up(&mut Playlist) -> bool` / `move_selected_down` | `Alt+Up` / `Alt+Down` | `false` at the list ends and with no selection |
| `remove_selected(&mut Playlist) -> Vec<EntryId>` | `Delete` (`1034`) | removed ids in playlist order (empty = no-op) |
| `crop_to_selected(&mut Playlist) -> Vec<EntryId>` | `Ctrl+Delete` (`1035`) | removed ids in playlist order (empty = no-op without a selection) |

Re-exported from the crate root (`lib.rs`).

## 3. Seam the window lane must call (spec, not an edit)

The window machines (`rustamp-ui::playlist::PlaylistWindow`, app routing in
`rustamp-app`) hold the order as `Vec<EntryId>` and emit core `Command`s; they
must not depend on this crate (S06i3 established that boundary deliberately).
Two equivalent seams, pick per surface:

1. **Command seam (app/engine path).** Translate the gesture into the already
   frozen wire commands and let the state owner apply them:
   - shift-click → `Command::Select(SelectOp::Set(ids))` with `ids` computed by
     `rustamp_playlist::range_ids(&entries, anchor, target)` (pure, takes a
     bare slice — ui may vendor-free call it only if it adds the dependency;
     otherwise replicate the clamp rule, which is three lines, and the
     `range_ids_matches_select_range` test pins what must match);
   - ctrl-click → `Command::Select(SelectOp::Toggle(id))`;
   - drag / `Alt+Up`/`Alt+Down` → `Command::MoveEntries { ids, to }` with the
     same `to` convention documented in §1 (index in the list *without* the
     moved entries; for `Alt+Down` from bounds `(first, last, count)` that is
     `last + 2 - count`);
   - `Delete` → `Command::Remove(selected_in_order)`; `Ctrl+Delete` →
     `Command::Remove(unselected ids in order)`.
   The app's applier (`App::apply_app`) already routes `Select`/`Remove`;
   `MoveEntries` must land on `Playlist::move_entries` (same convention, so
   the seam is consistent by construction).
2. **Model seam (whoever owns the `Playlist`).** Any caller holding the core
   model directly — engine-side model, tests, future IPC handlers — calls the
   `interact` functions themselves; they are the canonical semantics the
   command translation above must reproduce.

Anchor ownership: the **window** stores the shift-range anchor (entry id of
the last plain click / ctrl-click, reset by a plain click — see `click_select`
doc) and re-derives its index with `Playlist::index_of` / the synced order
after any reorder before calling `select_range`/`range_ids` again.

Pixel→row hit-testing, revealing the dragged row (`scroll_to`), and the
no-op-without-staged-selection UI feedback stay in the window half.

## 4. Tests (headless, written, **not run** here)

All in `interact.rs`'s `#[cfg(test)]` module (9 tests, pure model, no window,
no audio, no I/O):

| Test | Pins |
|---|---|
| `a_plain_click_selects_one_row_and_replaces_the_selection` | settled single-row selection replaces a prior range; past-the-end and empty refuse; current untouched |
| `a_shift_click_selects_the_contiguous_range` | forward and reversed range, replace-not-extend, both ends clamp to the last row |
| `a_ctrl_click_toggles_one_entry_and_keeps_the_rest` | toggle in/out beside a range; past-the-end no-op |
| `a_drag_moves_the_playing_entry_with_its_row` | reorder keeps current **identity** and moves its **index** with the row; block stays selected; permutation is loss-free; no-op reorder reports `false` |
| `alt_up_and_alt_down_move_the_selected_block_one_row` | one-row moves both ways with current identity + index after each; contiguous block moves as a block; refused at both ends; refused with no selection |
| `deleting_the_selection_shifts_the_current_index_by_what_preceded_it` | removed ids in order; surviving current's index shifts by exactly the removed predecessors (4→2); deleting the current clears it; unstaged delete is a no-op (S06 §5.3) |
| `crop_keeps_only_the_selection_and_removes_the_rest_in_order` | crop removes the complement in order, survivor index shifts, unstaged crop no-ops, cropping away the current clears it |
| `empty_and_single_entry_playlists_take_every_interaction` | every entry point on an empty list returns the refusal/no-op form; a single row answers a click, clamps any range, cannot be reordered, and deletes (clearing current) |
| `range_ids_matches_select_range_over_a_bare_id_list` | the two seam halves agree on every range including clamps and the empty list |

## 5. Files and line counts

| File | Change |
|---|---|
| `crates/rustamp-playlist/src/interact.rs` | new, 501 lines (module docs with per-rule spec grading, 10 functions incl. 2 private helpers, 9 tests) |
| `crates/rustamp-playlist/src/lib.rs` | module declaration, re-exports, two doc paragraphs (~+16 lines vs HEAD) |
| `docs/tasks/S06i4-playlist-interactions.md` | this card |

## 6. Verification and boundary receipt

- `cargo fmt --all -- --check`: clean (after one `cargo fmt --all` pass for
  import ordering).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo build --workspace --offline`: exit 0.
- `cargo test` was **not** run (suite runs in the Hyper-V test VM); the new
  tests were compiled via `clippy --all-targets` but **not executed**.
- No window opened, no audio played, no download, no `cargo add`, no new
  dependency, no git write, no commit.
- Writes went only inside this worktree. `git status --short` shows exactly:
  `M crates/rustamp-playlist/src/lib.rs`,
  `?? crates/rustamp-playlist/src/interact.rs`,
  `?? docs/tasks/S06i4-playlist-interactions.md`. Nothing under
  `D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin`
  was read or written.
