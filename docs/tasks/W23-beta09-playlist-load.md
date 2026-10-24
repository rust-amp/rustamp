# Task card W23 — BETA-09 (playlist-file loading): already closed by N7 — verification card

**Scope**: `crates/rustamp-app/**`, `docs/tasks/W23-beta09-playlist-load.md`.
**Write boundary**: this worktree only
(`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave105-beta09`). Nothing was
written under `D:\code\Rustamp` (proved clean at the end), and
`D:\code\rustamp-research` was never read.
**Verdict up front: BETA-09 is already implemented and needs no new code.** This card
records the proof (file, line, commit) and the one genuinely unpinned half (the CLI
path), which this lane pinned with a test. No behaviour was changed and no duplicate
was built.

## 1. The gap as it is quoted, and why the quote is stale

`docs/tasks/BETA-readiness.md:82-86` (P2 — BETA-09):

> **User impact today:** LIST OPTS opens `Load Playlist`, but the dialog shows an empty
> field and Enter has no load action. Passing a playlist as the CLI file also queues it
> as media, not as a list. No app playlist reader call is wired; this agrees with E3's
> caller audit.

This is a **point-in-time review**, not live status: the file's header pins it to
**Date 2026-10-17, reviewed revision `f9012dc`** (`BETA-readiness.md:3`), committed as
`0a06d66` (16:31). The fix for it is a later commit:

| Fact | Evidence |
|---|---|
| The review predates the fix | `git merge-base --is-ancestor 9e8c5d0 0a06d66` → **exit 1** (not an ancestor); `git log 9e8c5d0..0a06d66` is empty, i.e. the review commit is an ancestor of the fix |
| The fix commit | `9e8c5d0` "Make the load-playlist dialog actually load, and surface what was skipped", **2026-10-17 23:00:49** — 7 h after the review |
| It is in this worktree's history | `git merge-base --is-ancestor 9e8c5d0 HEAD` → exit 0; HEAD is `6946dd8`; 116 commits follow it |
| It is also in the canonical checkout | `git -C D:\code\Rustamp merge-base --is-ancestor 9e8c5d0 HEAD` → exit 0 (canonical HEAD `a1d0723`) |
| Independent re-check | `docs/tasks/N8-app-red.md:163-175` (§4 "BETA-readiness-2 BETA-09 re-checked: no longer true after N7"); `24b33e4` |
| The second review already says so | `docs/tasks/BETA-readiness-2.md:42`: BETA-09 **"LARGELY FIXED — Wired in N7 (`9e8c5d0`) and verified in N8 (`24b33e4`)… Residual: VM/user execution proof pending."** |

Consequence: the card's line cites (`playlist_window.rs:221-229`, `dialogs.rs:124-153`,
`dialogs.rs:202-205`, `app.rs:180-182`) are the **pre-N7** code. Two of them now *name
the fix*: `crates/rustamp-app/src/playlist_window.rs:222-231` is the LIST OPTS arm whose
`.expect("LIST OPTS has an opening command (RFC 0010)")` RFC 0010 demanded, and
`dialogs.rs:134-199` is the load-playlist key path. Three stale comments in this crate
still repeated the pre-N7 "does nothing" wording — corrected here (§5).

## 2. The four sub-claims, checked against today's code

| Card claim (revision `f9012dc`) | Today | Proof |
|---|---|---|
| "the dialog shows an empty field" | It shows a live entry field: prompt, the staged/typed path *tail*, then the hint; empty only while nothing has been typed or dropped | `crates/rustamp-app/src/dialogs.rs:285-317` (`dialog_body` → `load_playlist_lines`), `dialogs.rs:370-382`; pinned by `load_playlist_dialog_text_input_and_display` (`app.rs:4183`) |
| "Enter has no load action" | `Enter` confirms → `confirm_load_playlist` → `App::load_playlist_file(path, true)` → dialog closes; an **empty** confirm keeps it open | `dialogs.rs:157-161` (Enter), `dialogs.rs:192-199`; pinned by `good_playlist_file_loads_and_plays` (`app.rs:3966`) and `the_load_playlist_dialog_keeps_its_confirmation_open` (`app.rs:3379`) |
| "passing a playlist as the CLI file also queues it as media" | The CLI file is classified by extension: a non-playlist is queued as one media path, a playlist is loaded as a list | `app.rs:177-184` (`is_playlist_file`), `app.rs:224-228` (media branch), `app.rs:318-322` (playlist branch → `load_playlist_file(file, false)`); then `lib.rs:158-161` submits `Command::Play`, so `rustamp list.m3u` loads the list *and* starts it |
| "No app playlist reader call is wired" | Two call sites, both on the **reporting** reader | `app.rs:1677` and `app.rs:1740` call `rustamp_playlist::load_playlist_file_report` |

(`add_items` at `app.rs:1734-1785` additionally expands a playlist handed to
`Command::AddItems` through the same reporting reader, appending its survivors.)

## 3. Which reader the app calls, and what the legacy readers lose

`rustamp-playlist` exposes both shapes (read-only citation; that crate is another lane's):

- reporting: `load_playlist_file_report` (`crates/rustamp-playlist/src/io.rs:82`),
  returning `PlaylistReport { entries, skips }` (`entry.rs:90`) with `SkippedEntry`
  reasons (`entry.rs:62`, `entry.rs:76`), `is_clean` / `skip_count`
  (`entry.rs:104`, `entry.rs:109`);
- legacy: `load_playlist_file` (`io.rs:77`) and friends, which delegate to the report and
  call `.into_entries()` (`entry.rs:114`) — the line that **discards every reason**.

The app calls **only the reporting API**, so E3's retained reasons survive into the app.
E3 is therefore not wholly unfixed: it built the reasons and the report surface; what was
missing was a caller, and N7 supplied it (`docs/tasks/E3-playlist-skips.md:87-106`
recommends exactly this: `load_playlist_file_report`, then surface `report.skips`).

## 4. What happens today, entry by entry

**Dialog (LIST OPTS)** — `playlist_window.rs:222-231` submits `Command::OpenPlaylist`
(kept by RFC 0010), the app opens `DialogKind::LoadPlaylist`; the field takes OS text
(`window.rs:864-901` → `App::dialog_text`), Backspace, and a shell-dropped file
(`window.rs:913` → `dialog_drop_file` → `drop_playlist_path`); `Enter` confirms
(`dialogs.rs:192-199`). Acceptance is text-field based (`rustamp-ui/src/dialogs.rs:520`,
`540`) and confirmation reads the staged pick or the trimmed text
(`rustamp-ui/src/dialogs.rs:676`).

**CLI** — `rustamp mix.m3u`: `is_playlist_file` true → not queued as media
(`app.rs:224-228`) → `load_playlist_file(file, false)` (`app.rs:318-322`) replaces the
queue with the surviving tracks in file order → `lib.rs:158-161` (or `:134-136` with
`--null`) submits `Command::Play`, so it starts. `rustamp song.mp3` still takes the media
branch.

**A dropped entry** (dangling `#EXTINF`, unresolvable location, empty/orphaned
`FileN=`, XSPF track without a location — E3 §2's table):

1. it is absent from the queue (the reader dropped it);
2. the reader's reason survives into `App::last_playlist_report`
   (`app.rs:373-375`, filled at `app.rs:1730`);
3. it is surfaced as notes (`app.rs:1699-1709`): one summary line
   (`loaded N tracks, skipped M malformed entries from <path>`) and one line per reason;
   notes are buffered for headless `drain_notes()` and printed to `stderr` in windowed
   runs — so it is no longer silent, though the visible surface is stderr, not a dialog.

**A file that cannot be parsed at all** leaves the queue untouched, sets `last_error`
(shown in the native title bar), notes it, and publishes `Event::Error`
(`app.rs:1679-1697`).

## 5. What this lane changed (verification and hygiene only — no behaviour change)

1. **Added** headless test `playlist_file_given_on_the_command_line_loads_as_a_list`
   (`crates/rustamp-app/src/app.rs:4085`). This is the one half of BETA-09 that **no test
   pinned**: the dialog half has four tests, the CLI half had none.
2. **Corrected three comments that still asserted the pre-N7 behaviour** — the exact
   wording that makes the readiness card look current:
   - `window.rs:855-863`: "a no-op on the load-playlist … dialogs, whose confirmation is
     open per RFC 0010" → now states Enter confirms and loads, and names preferences as
     the remaining no-op window;
   - `app.rs:3380-3383` (test comment "RFC 0010: no confirmation semantics. Enter is
     consumed and changes nothing") → describes the empty-confirm rule it actually tests;
   - `app.rs:2401-2404` ("implements no dialog behaviour beyond it — in particular no
     note") → the command opens the window; the dialog's behaviour is pinned below.

No production code path changed, no contract was touched, and nothing outside
`crates/rustamp-app/**` was edited. Diff: `app.rs` +63/-4 lines, `window.rs` +7/-6 lines.

## 6. The test added, and how it can fail

`playlist_file_given_on_the_command_line_loads_as_a_list` writes an M3U with a dangling
`#EXTINF` plus two good entries, passes the **path as the CLI `<file>`** through
`AppOptions`, and asserts: the queue holds exactly the two resolved survivor paths in
order; the playlist path itself is *not* in the queue; the recorded report is not clean,
has 1 skip containing `dangling #EXTINF`, and no error; and the notes contain the summary
and the per-reason line. It fails if any of the three load-path properties regress:

- **CLI classified as media again**: delete the `is_playlist_file` branch at
  `app.rs:318-322` → the queue becomes one entry (the `.m3u` path) → `len() == 2` and
  `!queue().contains(&TrackSource::path(m3u))` both fail.
- **The app swapped to the legacy reader** (`rustamp_playlist::load_playlist_file`,
  `io.rs:77`): `last_playlist_report()` is `None` → `.expect("report recorded")` panics;
  the skip assertions are unreachable. This is precisely the card's silent-drop boundary.
- **Surfacing removed** (`app.rs:1699-1709`): the two note assertions fail.

## 7. What is still open — as questions, not invented answers

RFC 0010 `:88-100` and S06 §4 settle the dialog's **title** only; N7's card
(`docs/tasks/N7-playlist-load.md:111-123`) records its own choices. These stay questions:

1. **Replace or append?** N7 chose *replace* for the dialog and the CLI, *append* for
   `AddItems`. RFC 0010 `:88-93` explicitly refuses to settle it ("confirmation is *not*
   mapped to `AddItems`"). No spec measurement exists; the behaviour is a choice.
2. **Does confirming the dialog start playback?** N7 chose yes (`play: true`,
   `dialogs.rs:196`); the CLI reaches Play through `lib.rs:158-161`. Neither spec nor RFC
   fixes it.
3. **Should skips be visible in the window instead of stderr?** E3 §4.2 says "if UI
   notification is supported"; N7 had no status line and used notes. Whether the beta
   needs an in-window line (BETA-09's own "surfaced rather than lost") is a product call.
4. **`.b4s`**: `is_playlist_file` uses `PlaylistFormat::ALL`, which contains `B4s`
   (`rustamp-playlist/src/format.rs:39-44`), and B4S is recognized-but-unsupported
   (`format.rs:60-65`). So `rustamp x.b4s` now fails as an unloadable *playlist* (clear
   message, queue untouched) instead of as an unplayable *media* file. Whether that is the
   wanted classification, or `.b4s` should be excluded from `is_playlist_file`, is open.
5. **A playlist whose every entry is dropped** yields an empty queue and then a `Play`
   from `lib.rs` that the engine rejects with an `Event::Error` (surfaced by the BETA-04
   path). Whether the app should skip the Play for an empty confirmed load is open.

Any of these that needs a library change is out of this lane's scope (owned paths:
`crates/rustamp-app/**` only).

## 8. Verification receipt

- `cargo fmt --all -- --check`: **exit 0** (first run exit 1 on one line of the new test;
  re-run after re-wrapping: 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **exit 0**, no
  warnings — this also compiles the new test target.
- `cargo build --workspace`: **exit 0**.
- `cargo test`: **not run** (lane constraint; the suite runs in the orchestrator's Hyper-V
  VM). No window was opened, no VM job was started, no audio was played, no dependency was
  added, no download, no `git` write, no commit.
- Files changed, all inside the assigned worktree:
  `crates/rustamp-app/src/app.rs` (+63/-4), `crates/rustamp-app/src/window.rs` (+7/-6),
  plus this card (new).
- `git -C D:\code\Rustamp status --short`: empty (0 lines). It was briefly non-empty
  during this run — `docs/notes/lane-jobs.md`, `docs/rfc/0012-output-error-hook.md`,
  `docs/tasks/LANE-QUEUE.md`, all with mtime `18.10.2026 17:53:20` — which is the
  orchestrator's own lane bookkeeping being written in its own checkout, not this lane:
  no tool call here ever wrote outside the worktree, and the status returned to empty on
  re-check. This lane's own writes are exactly the three files listed above.

## 9. What only the VM can prove

1. The four pre-existing N7 tests and the new CLI test **executing** green — the host may
   not run `cargo test`.
2. The windowed end to end: LIST OPTS → `Load Playlist` window → typed path or dropped
   file → `Enter` → the playlist window and the main window follow, and the skip lines
   appear on stderr.
3. `rustamp <playlist>` on a real launch (GUI rail): the list loads and the first track
   plays.
4. Whether the composed queue plays real audio (needs a device): untouched by this lane.

## 10. Provenance

- Implemented and verified only from `specs/` (S06 §4), `docs/rfc/0010-load-playlist-command.md`,
  `docs/tasks/E2-playlist-meta-audit.md`, `docs/tasks/E3-playlist-skips.md`, the tasks cards
  cited above, and clean-room Rustamp code. No Winamp source, SDK headers, documentation or
  disassembly were read; `D:\code\rustamp-research` and `D:\code\rustamp-refbin` were not
  accessed; the canonical tree `D:\code\Rustamp` was read read-only (the `git` queries in §1
  and the final status proof) and never written.
- Third-party code copied: none. License impact: none (no dependency change;
  `rustamp-app` stays GPL-3.0-or-later).
