# W23 — BETA-07 playlist visibility / check-mark reconciliation

**Status:** **Gap already closed — no source change required.**
**Verdict date:** 2026-10-18. **Head reviewed:** `964d2d1` (`HEAD` of the
beta07 worktree). **Snapshot card:** `docs/tasks/BETA-readiness.md`,
revision `f9012dc` (2026-10-17 16:21:17).

## 1. The exact lines the card gives

`BETA-readiness.md` §2 (P2 — BETA-07) states:

> the playlist opens even though the initial model says closed; its toggle
> changes a flag/check but cannot hide it, and its native close explicitly
> keeps it open (`crates/rustamp-ui/src/state.rs:145-146`,
> `crates/rustamp-ui/src/behaviour.rs:410-413`,
> `crates/rustamp-app/src/window.rs:1198`,
> `crates/rustamp-app/src/window.rs:1071-1075`). There is no playlist
> counterpart to EQ's visibility-controlled creation/destruction
> (`crates/rustamp-app/src/window.rs:376-385`,
> `crates/rustamp-app/src/window.rs:1001-1037`,
> `crates/rustamp-app/src/window.rs:1336-1354`).

Every one of those line numbers belongs to the **snapshot revision** `f9012dc`,
which is ~1 h older than the fix (see §3). The current tree has restructured
the file; the references map to dead code that no longer exists. The card is
stale on every limb.

## 2. Verdict: the gap is not real today

The BETA-07 finding has four limbs. Each is **already false** against HEAD `964d2d1`;
the evidence is file, line and commit.

| BETA-07 limb | Refuted by (current file:line, HEAD) | Commit |
|---|---|---|
| **1. "playlist opens even though the initial model says closed"** | `crates/rustamp-app/src/app.rs:259` — `playlist_open: pl_layout.map(\|l\| l.visible).unwrap_or(true)`; the App seeds the model **open**, matching the window the presenter opens in `resumed` (`window.rs:1506`). The UI's `MainWindowState::default` keeps `playlist_open: false` (`state.rs:149`) on purpose and is always overridden at construction. | `b6d1d5f7` (2026-10-17 19:06) |
| **2. "toggle changes a flag/check but cannot hide it"** | `behaviour.rs:462-463` flips `playlist_open`; `window.rs:1135-1143` `sync_playlist_window` creates or **drops** the OS window to match the flag (same pattern as EQ at `window.rs:426`); it is called every frame in `about_to_wait` (`window.rs:1705`) and immediately after any playlist-pointer input (`window.rs:1309`). A PL-toggle release on the main window flows: `mouse_up` → flag flips → `Command::ToggleWindow(Playlist)` → `sync_window_state` → next `about_to_wait` → `sync_playlist_window` drops/creates the window. | `4bf70262` (2026-10-17 17:03) |
| **3. "native close explicitly keeps it open"** | `window.rs:1233-1239`: the playlist window's `CloseRequested` calls `self.app.close_playlist_window()` (which submits `ToggleWindow(Playlist)`, `playlist_window.rs:285-292`) then `self.sync_playlist_window(event_loop)` to drop the OS window. Closing the playlist never quits — `App::quit` stays false (verified by `first_minute.rs:412`, `playlist_window.rs:553`). | `4bf70262` (2026-10-17 17:03) on the CloseRequested arm from `7f612e0` (2026-10-16 11:20) |
| **4. "no playlist counterpart to EQ's visibility-controlled creation/destruction"** | `sync_playlist_window` (`window.rs:1135-1143`) is the direct counterpart of `sync_eq_window` (`window.rs:426-497`): both create-or-drop from an open flag, both are called from `resumed` (`:1506`/`:1499`), `about_to_wait` (`:1705`/`:1688`), and after a right-click (`:1650`). | `4bf70262` (2026-10-17 17:03) |

**Commit ancestry:** `7f612e0`, `4bf70262`, and `b6d1d5f7` are all ancestors
of `HEAD` `964d2d1` (verified with `git merge-base --is-ancestor`, all true).

## 3. Why the card went stale — timing

`BETA-readiness.md` is pinned to `f9012dc` (2026-10-17 16:21:17, "Draft RFC
0012"). The playlist-visibility wiring landed **after** that pin:

- `7f612e0` 2026-10-16 — "Add the playlist window (lane)" (N5): initial
  playlist window, `CloseRequested` arm, first tests.
- `4bf70262` 2026-10-17 17:03:46 — "Deliver the first minute: open a file
  from the GUI, stop phantom playback, make the visible controls act" — this
  is the BETA-06/BETA-07 delivery: `sync_playlist_window`,
  `close_playlist_window`, the toggle command arm (`app.rs:1566-1569`, the
  no-op the presenter owns), and the main-window toggle routing.
- `b6d1d5f7` 2026-10-17 19:06:13 — "Make the configuration real: honoured
  settings, a written file, loud malformed input" — sets
  `playlist_open: …unwrap_or(true)` so the model starts open, closing limb 1.

The snapshot was therefore taken before two of the three fixing commits existed.
This is the same pattern the readiness review already found for BETA-08
(shuffle/repeat), BETA-09 (playlist loading) and the second golden set —
all fixed after the snapshot. A lane that reports "no change needed" with
commit + line proof is correct here.

## 4. What the specs settle and what they leave open

**Settled (relevant to BETA-07):**

- The playlist editor is a **toggle window**: closing it hides rather than
  quits (`RFC 0005` `CloseFocusedWindow`; `S03 §2` Ctrl+F4 "Close the playlist
  editor"). Implemented in `playlist_window.rs:271,285-292` and the native
  `CloseRequested` arm at `window.rs:1233-1239`.
- The PL button, the menu row, and the playlist's own Close box all flip the
  same `playlist_open` flag (`behaviour.rs:462-463`, `menu.rs:170`,
  `render.rs:415`).
- S05 §3.3/§4.2: the playlist is a **docked neighbour** of the main window and
  tracks the main's bottom edge (`window.rs:1205-1219`, `sync_playlist_position`,
  `playlist_window_origin` at `app.rs:65-70`).
- The playlist never double-sizes (S05 §2.1): its client area is always
  275 x 116 (`window.rs:1146-1158`, `PLAYLIST_WINDOW_SIZE`).

**Open (not a BETA-07 blocker):**

- S05 §1.5: the playlist editor's **windowshade** is unmeasured; the shade
  button records a note instead of acting (`playlist_window.rs:272-278`).
  Correct as-is — no spec to implement against yet.
- S05 §4.1: snap rules (edges, tolerance) are unmeasured.
- S05 §2.2: the stack's positions in plain double-size are unmeasured.
- S05 §3.4: persistence across restart is unmeasured (and BETA-12 records the
  config save path is not yet production-wired).

None of these opens affect whether the PL toggle / check mark / native close
control the OS window. That contract is fully resolved.

## 5. What was changed here

**Nothing.** The gap is already closed in the tree. A "no change needed"
reconciliation card is the honest deliverable: inventing a duplicate
`sync_playlist_window`, re-adding a `playlist_open` override, or "fixing" the
`MainWindowState::default` `false` (which is intentionally overridden by the
App) would regress a working design, not improve it.

No file under `crates/rustamp-*` was modified. No dependency was added. No
commit was made.

## 6. Existing tests that already prove the marks tell the truth

Headless tests (not run on the host; they compile under `--all-targets` and
run in the Hyper-V test VM). Each can fail in a way that falsifies "the marks
tell the truth":

| Test | File | What it pins | How it can fail |
|---|---|---|---|
| `the_playlist_close_hides_it_and_the_pl_button_brings_it_back` | `crates/rustamp-app/tests/first_minute.rs:404` | starts open; Close at (264,3) emits `ToggleWindow(Playlist)`; flag goes false; `quit()` false; PL button reopens | fails if Close quits, or if close/toggle don't flip the flag, or if PL doesn't reopen |
| `the_title_bar_close_hides_the_editor_and_pl_reopens_it` | `crates/rustamp-app/src/playlist_window.rs:544` | title-bar Close emits `ToggleWindow(Playlist)`; flag false; no-op on second close; resubmit reopens | fails if Close is a no-op, quits, or double-close panics |
| *(menu check at launch)* | `crates/rustamp-app/tests/main_menu.rs:80-81` | `assert!(app.menu_check(PlaylistEditor))` at launch — the check mark matches the open window | fails if the check mark disagrees with the window state at launch |
| *(menu toggle)* | `crates/rustamp-app/tests/main_menu.rs:94-95` | after menu activate, `playlist_open == false` and `menu_check == false` | fails if the menu toggle doesn't flip the check |
| *(config visibility)* | `crates/rustamp-app/tests/config.rs:357` | config `visible: false` → `playlist_open() == false` | fails if config visibility is ignored |
| *(shortcut toggle)* | `crates/rustamp-ui/src/shortcuts.rs:1387-1390` | shortcut emits `ToggleWindow(Playlist)` and flips `playlist_open` | fails if the shortcut is dead or flips the wrong flag |
| *(widget emits command)* | `crates/rustamp-ui/src/behaviour.rs` (`every_interactive_widget_emits_its_command`) | `Widget::PlaylistToggle` emits `ToggleWindow(Playlist)` | fails if the button emits nothing or the wrong command |

The check-mark truth condition — "a check mark must mean the window state it
names" — is pinned at `menu.rs:170` (`MenuCheck::PlaylistEditor =>
state.playlist_open`) and exercised by `main_menu.rs:80-81,94-95`: the mark
reads the very flag the presenter turns into a window. All three surfaces
(PL button render, menu check, window presence) read `playlist_open`, so they
cannot disagree.

## 7. What only the virtual machine can prove

Static source cannot prove the OS-window outcome end to end:

- That `event_loop.create_window` for the playlist (window.rs:1169) actually
  surfaces a 275 x 116 `Winamp PE`-class window that the presenter draws into.
- That `about_to_wait`'s `sync_playlist_window` (window.rs:1705) runs on the
  frame after a PL-toggle release on the **main** window (the main mouse
  handler at `window.rs:1630` calls `sync_window_state` + `sync_dialog_window`
  but not `sync_playlist_window` directly — it relies on the per-frame
  `about_to_wait` tick, the same deferred sync the equalizer uses). This is
  consistent but is a timing claim no headless test can assert.
- That the native playlist CloseRequested (`window.rs:1233-1239`) hides the OS
  window and does not fire the main-window quit path.
- That the docked offset (`window.rs:1205-1219`) keeps the playlist flush
  below the equalizer across a real title-bar drag (S05 §4.2).

The `tests/main_menu.rs` and `tests/first_minute.rs` cases are compiled by
`clippy --all-targets` and run in the VM; they exercise the App model (flag
flip + command), not the OS window.

## 8. Verification and boundary receipt

- `git status --short` (inside this worktree): `?? docs/tasks/W23-beta07-playlist-visibility.md`; no tracked source modification.
- `git status --short` in `D:\code\Rustamp`: empty (never wrote there).
- No download, commit, git write, new dependency, or code change was made.
