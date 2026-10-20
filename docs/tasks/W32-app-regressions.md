# W32 — App guest regressions after the native shell/menu changes

## Scope and rules

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave132-appvm`.
- Write only `crates/rustamp-app/**`, `crates/rustamp-ui/**` and this card;
  nothing under the canonical `D:\code\Rustamp` tree.
- No downloads, dependencies, commits or other git writes. No GUI on the host.
- Keep the borderless shell and the OS-drawn native menus. Do not weaken a
  product assertion to accept the former defects.
- Sources: repository implementation, audited S01/S04/S05/S10 specs, and the
  installed freely licensed winit 0.30.13 source (event-loop lifetime rule).
  No Winamp source, SDK, disassembly or research repository was read. No
  third-party code was copied and no contract changed.

## 1. External-control startup timeout

**Established test defect; guest root cause still requires a rerun.**
`tests/headless.rs::external_control_drives_the_player_window_in_the_test_vm`
sends `WM_WA_IPC` directly to the child player's main HWND. This is **not** the
player-to-plugin-host transport: `src/external_control.rs` and its `native.rs`
subclass decode/answer version id 0 locally; neither needs a plugin host.

`Windowed::resumed` creates a visible main window before constructing its
softbuffer surface, attaching the tray, and attaching external control. Finding
that HWND therefore does not prove message/subclass readiness. The existing
`eventually` loop intended to wait for attachment, but its inner `query` panicked
on the first failed 1000 ms send, bypassing the outer 10 s readiness deadline.

The new startup barrier retries unsuccessful/mismatched version reads only
within that existing 10 s budget, checks that the child has not exited, and
requires the exact S10 version `0x2905`. All subsequent sends, including an
additional version read immediately after readiness, retain the original hard
1000 ms timeout. Setter non-mutation and posted-command state checks remain.
The guard kills/reaps the child on panic and prints saved stdout/stderr instead
of throwing them away. A permanently unresponsive window, a missing subclass,
a wrong reply or a dead player still fails loudly. No product change was made
on the unsupported assumption that the guest timeout is only a flake.

## 2. Playlist shade / `RecreationAttempt`

**Test-process lifetime defect, not an output cleanup/recreation failure.**
The reported line unwraps `build_event_loop(true)`, whose error type is
`winit::error::EventLoopError`. In installed winit 0.30.13,
`src/event_loop.rs:118–119` returns `RecreationAttempt` when the process-global
`EVENT_LOOP_CREATED` was already set; its reset is web-only (lines 130–133).
The docking and shade lib tests both constructed an event loop in the same
test executable. The guest runner uses `--ignored --test-threads=1` per binary,
not a new process per test; serial execution does not reset winit's latch.
The shade test can therefore fail before it creates a window or clicks shade.

Every lib-test window probe now reexecutes its exact test in a fresh harness
process, with `CREATE_NO_WINDOW`, a 30 s outer timeout, mandatory successful
exit and proof that exactly one test ran/passed. No latch reset, skip or error
acceptance is used. Both original shade transitions remain: client 275×116 →
275×14 → 275×116 with unchanged outer origin. Each read additionally requires
outer size == client size, so a reintroduced native frame also fails.
The docking probe still runs its full production-presenter assertions.

## 3. Two menu probes reported NOT RUN

**Deleted test coverage / stale expected inventory, not a guest menu failure.**
`docs/notes/vm-run-tests.ps1:791–800` still inventories the two original names.
W24 explicitly records deleting both probes after switching to the native
popup. The runner cannot execute absent tests and correctly prints NOT RUN.
The solution is to restore the tests under those exact names, not delete the
inventory entries. No runner file needs changing.

- `window::tests::the_menu_popup_opens_at_the_measured_size`: runs the real
  `Windowed::resumed` → `Command::OpenMainMenu` → `drain_main_menu` path.
- `the_main_menu_popup_opens_at_the_measured_client_size` in `tests/main_menu.rs`:
  passes the app's real native tree through `show_popup_menu` and checks that
  cancellation returns no choice. Despite its retained historical name, this
  no longer asserts a skin-painted client-window size.

Both are ignored by default, gate on the test VM before spawning/creating
anything, and use the same Win32 readback helper. While the real system menu
is modal, an observer finds exactly one visible `#32768` window on the owner's
thread and reads the real HMENU through `GetMenuBarInfo(OBJID_CLIENT)`. It checks:

- exact menu-button screen anchor, real `WS_POPUP`, no `WS_CAPTION`;
- actual item count, every live label/accelerator string, separator flags,
  submenu handles and checked state, and no owner-drawn bitmap text;
- positive actual screen rectangles for every row, contiguous nonoverlapping
  row bands, all rows inside the popup and its independently read menu bounds.

The popup/menu/row-union rectangles are logged. The OS controls font metrics,
theme and DPI; S04's 323×569 screenshot was recorded at 150% scale, so forcing
that historical rectangle on a system-sized menu in a 100% guest would assert
the wrong thing. These probes instead catch the former caption/bitmap-menu
defect, missing/wrong rows, clipped rows and wrong placement in the actual OS
output. Existing headless S04 tree tests remain unchanged. Win64 readback
struct layouts are additionally tested headlessly in both consuming binaries.

## Verification and remaining guest verdicts

Final host gates (2026-10-20), each exit code captured immediately after Cargo:

| Gate | Exit | Receipt in this worktree |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | `target/w32/fmt-final.log` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | `target/w32/clippy-final.log` |
| `cargo test --workspace --all-features` | 0 | `target/w32/test-final.log` |

The full test log reports 2466 passed, 0 failed, 47 ignored across 142 result
blocks (including doc tests). Both new readback ABI checks passed; all four
subject GUI probes stayed ignored. `CARGO_NET_OFFLINE=true` prevented downloads.
An earlier combined run reached fmt/clippy 0 but was interrupted by the shell's
120 s limit during the full suite; the final run above had no shell timeout.
`git diff --check` exited 0. A final read-only
`git -C D:\code\Rustamp status --short` exited 0 with empty output. No commit
or other git write was made, and no file outside the owned app paths/card was
changed (the UI crate needed no change).

The compiled lib harness's `--list --ignored` lists the restored
`window::tests::the_menu_popup_opens_at_the_measured_size` (7 ignored cases in
that binary); the `main_menu` harness lists
`the_main_menu_popup_opens_at_the_measured_client_size` (1 ignored case).
Both listing commands exited 0 without executing a window test. Their exact
inventory names therefore resolve again; this is not a claim they passed.
No ignored window probe is run on the host. Only an **interactive guest** run
can confirm startup responsiveness, the real shaded/restored HWND sizes and
origin, and the actual native-menu Win32 geometry/dismissal assertions. A host
compile or inventory listing is not a passing VM verdict. Human/pixel acceptance
of OS font legibility is not replaced by structural/menu-rectangle readback.

For detailed guest receipts use the compiled test harnesses with
`--ignored --nocapture --test-threads=1` in the interactive VM (the general
ignored suite must include both lib window tests in one parent process, which
is the regression that used to trigger winit's recreation refusal). The IPC
probe logs retry count/readiness time, and every popup probe logs its actual
screen/menu/row-union rectangles. Per-test success alone is not proof of the
multi-probe process-lifetime fix.
