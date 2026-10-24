# W24 — BETA-07: Playlist visibility regression coverage

Status: **Original finding stale by current source inspection; replacement tests
await VM execution.** No production changes are needed. The abandoned test did
not compile; its earlier gate-success claims were invalid and are superseded
by the receipts below.

## Scope and boundaries

- Write only this card and `crates/rustamp-app/tests/playlist_visibility.rs` in
  `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave119-beta07`.
  Write nothing under `D:\code\Rustamp` or any other directory.
- Do not change contracts, dependencies, other lanes' crates, or Git state.
- No tests, GUI, VM, VM broker, reference binaries or downloads in this lane.
  Test execution belongs to the orchestrator's interactive VM session.
- Initial `git status --short` was empty. Contrary to the recovery prompt's
  expectation of two untracked files, both abandoned files were already tracked
  (`git ls-files` listed both). Their existing contents were reviewed in full.

## What was retained and what was replaced

Retained: the finding that BETA-07 in `docs/tasks/BETA-readiness.md` describes
an old snapshot, and the intended regression properties: initial visibility,
close-versus-quit, live checks, all toggle inputs, and restored hidden state.
Earlier implementation is documented under N5 (`4bf7026`) and configuration
restoration (`b6d1d5f`); this lane verifies the current code, not a reference run.

Replaced: the uncompilable two-test file, not the production implementation.
`MenuOutcome` belongs to `rustamp_app::menu`, not `rustamp_ui`;
`AppOptions::headless`, `MainWindowLayout::default`, `layout.widget_rect`,
and `App::dispatch_shortcut` do not exist. `WindowLayout` dimensions are
optional, not bare integers. The replacement uses an explicit null-output
`AppOptions`, `Widget::rect(&MainWindowLayout::CLASSIC)`, `App::key_down`,
the real menu row, and the production TOML parser.

Removed: unsupported claims that a headless test proves OS window destruction
or checks `playlist_window.is_none()` (it has no access to that field), and
that the old hand-constructed `Config` tested parsing. Also removed inaccurate
spec generalizations: S04 §3.1 explicitly leaves the check marks' meaning
unestablished. It observes the fresh-launch glyphs; the live-state relationship
here is the existing Rustamp regression contract, not a new measured fact.

## Current-source evidence (production unchanged)

| Property | Evidence |
|---|---|
| Default open, stored visibility honored | `crates/rustamp-app/src/app.rs:242-259`: find the Playlist layout by id; `visible` overrides `unwrap_or(true)` |
| Shared live flag | `crates/rustamp-app/src/app.rs:363-364`: `playlist_open()` reads the UI state; `crates/rustamp-ui/src/behaviour.rs:462-463` toggles it |
| Live menu check | `crates/rustamp-app/src/menu.rs:74-76` calls `check_state` on current state; `crates/rustamp-ui/src/menu.rs:170` reads `state.playlist_open` |
| Real menu binding | `crates/rustamp-ui/src/menu.rs:344-351`: Playlist row, check and toggle action; `crates/rustamp-app/src/menu.rs:138-153` forwards its commands |
| Mouse and Alt+E | `crates/rustamp-ui/src/behaviour.rs:499`, `crates/rustamp-app/src/app.rs:1170-1194`, `crates/rustamp-ui/src/shortcuts.rs:420-425` |
| Skinned close and native-close app entry point | `crates/rustamp-app/src/playlist_window.rs:269-291`: Close delegates to an idempotent helper submitting `ToggleWindow(Playlist)` |
| Presenter creation/destruction | `crates/rustamp-app/src/window.rs:1135-1142`: create when open; drop surface/window when closed |
| Native event routing | `crates/rustamp-app/src/window.rs:1233-1238`: CloseRequested invokes the helper and immediately synchronizes |
| Pointer, startup and frame synchronization | `crates/rustamp-app/src/window.rs:1309,1506,1705`: synchronization after pointer input, at resume and on the frame pump |

These are source-inspection findings, not executed desktop acceptance evidence.

## Replacement tests and concrete failure modes

All eight tests use null output and the built-in skin, with no queued media,
child processes, event loops or OS windows. The shared assertion checks
`playlist_open()`, the main-window render flag, the live menu check and `!quit()`.

| Test suffix (`beta07_…`) | What makes it fail |
|---|---|
| `default_launch_starts_open_and_checked` | Fresh App starts closed, unchecked, or quitting |
| `skinned_close_emits_a_toggle_not_quit` | Close hit-testing stops emitting exactly one Playlist toggle, the model stays open, the check stays on, or Quit is requested |
| `native_close_entry_point_hides_once_and_never_reopens` | The app helper stops emitting the toggle, leaves the model/check open, quits, or toggles back on a second close |
| `left_click_pl_toggles_visibility_and_check_both_ways` | Press/release stops emitting exactly one toggle, applies it twice, or either direction/check is broken |
| `right_click_pl_toggles_visibility_and_check_both_ways` | Right-click does not emit/apply the Playlist toggle in either direction |
| `real_menu_row_toggles_visibility_and_check_both_ways` | The shipped row has the wrong check/action, or activating that row's actual action fails either direction |
| `alt_e_key_lookup_and_dispatch_toggle_both_ways` | The actual Alt+E lookup, command emission, forwarding, or live check fails; direct shortcut dispatch is not substituted |
| `parsed_playlist_visibility_overrides_default_and_other_windows` | Production `parse_with_warnings` loses true/false, App ignores it or takes the preceding Main layout with opposite visibility, or the live toggle/check stays stuck at the saved value |

The configuration case parses literal TOML in memory through the same parser
used by `config::load`; it does not claim disk I/O or save-on-exit coverage.
Left/right mouse, menu and Alt+E each run closed then open on a fresh App.
The restored-layout case tests both visibility values and then toggles each.

## Verification receipts

Recovery gates ran on 2026-10-18 in this worktree, offline, with build output
and temporary files directed to its own `target/`. Every exit code was captured
immediately into a PowerShell variable before any subsequent command.

- `cargo fmt --all -- --check`: **exit 0** (`$fmtExit`).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **exit 0** (`$clippyExit`); includes type-checking all eight regression tests.
- `cargo build --workspace`: **exit 0** (`$buildExit`).
- `cargo test`: **not run**, as requested. No VM or broker invoked.

Local untracked/ignored receipts: `target/beta07-gates/{fmt,clippy,build}.log`,
`fmt-exit.json` and `compile-exits.json` in the same directory. These replace,
rather than repeat, the abandoned lane's unverifiable success claims.

## What only the virtual machine can prove

- Execute these eight tests and the workspace suite in the interactive session;
  host Clippy type-checks tests but does not run assertions, and build does not
  replace that execution.
- Verify native CloseRequested actually reaches the app helper, destroys the
  playlist HWND without quitting the player, and allows repeated reopening.
- Verify model-to-window synchronization at startup (including hidden config),
  after real mouse/menu/keyboard input, and after every hide/show cycle.
- Verify softbuffer/winit surface recreation, real painted check marks, focus,
  stacking and docking. The existing ignored `tests/playlist_window.rs` checks
  opening/geometry; it is not a substitute for a hide/show/native-close probe.

Sources: own project code and tests, `specs/S03-shortcuts.md` §1/§5 and
`specs/S04-menus.md` §2/§3.1. No Winamp source, SDK, disassembly, research
repository or copied third-party code was read or used. No dependency changes.
