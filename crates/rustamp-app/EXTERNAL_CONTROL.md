# Player-window external control (S10)

The **real player main window** uses the class `Winamp v1.x`. This adapter
does not change the separate 32-bit plugin host. Only the main HWND is
subclassed; EQ/playlist HWNDs do not forward IPC. Existing window-text/title
updates remain available through normal Windows window-text APIs.

Queries are synchronous `WM_WA_IPC` (`0x0400`): ID in `LPARAM`, argument in
`WPARAM`. Classic arguments/results use their low 32 bits; signed pan is
returned in unsigned 32-bit form. Queries read the latest player snapshot,
published after each UI-thread pump (normally every 16 ms), not fixture
defaults. Commands are queued and drained before the next pump; neither a
dialog nor application code runs inside the native callback.

## Coverage

S10 is still a revised draft. This implementation uses its explicit measured
rows, not the plugin host's additional provisional fallbacks. An unsupported
call returns **1**. That is a capability fallback, **not** a claimed reference
measurement for known-but-unsettled IDs/states.

| S10 ID | Status | Boundary / source |
|---|---|---|
| 0 | Implemented | `0x2905`, §3.1; in-process reference cross-check remains open |
| 104 | Implemented | Live stopped/playing/paused = 0/1/3; argument ignored |
| 105, modes 0/1/2 | Partially implemented; needs spec | Fresh stopped/no-list/no-stream = -1/-1/0. Playing/paused and loaded-state semantics/units are open (§13.2); return unsupported there |
| 106 | Partially implemented; needs spec | Stopped = -1; seeking while playing is open (§13.2), no seek invented |
| 109 | Partially implemented; needs spec | Fresh no-stream = 0; loaded-module distinction remains open |
| 122, argument -666 | Implemented | Live unsigned volume |
| 123, argument -666 | Implemented | Live signed pan, including -1 as `0xffffffff` |
| 124, 125 | Implemented | Live list length/current index; empty position = 0 |
| 126, modes 0/1/2 | Partially implemented; needs spec | Fresh stopped/no-stream = 0. Playing stream-info units/values open (§13.2) |
| 127, selector 99 | Implemented | Measured invalid selector = 0 |
| Other 127 selectors; 128 | Needs spec | S10 establishes stateful raw slots, but does not specify all numeric selectors or raw-slot ↔ player dB conversion. No duplicate raw EQ state |
| 102 | Already handled by command path (posted) | `Command::Play`; sent reply open |
| 122, posted set 0..255 | Already handled by command path (posted) | `Command::SetVolume`; sent reply open (§13.11) |
| 123, posted set 0/64/255 | Already handled by command path (posted) | `Command::SetBalance`, 255 → -1. Other inputs need measurement (E9) |
| 250, 251 | Implemented | Live shuffle/repeat |
| 252, 253, posted set 0/1 | Already handled by command path (posted) | `Command::SetShuffle` / `Command::SetRepeat`; no invented -666 query |
| 400 | Partially implemented; needs spec | Fresh stopped = 0; other states not established |
| 502, 666, 801, 999, 3015, 3025, 65536 | Implemented | Measured reply 1; no inference from job-derived names |
| 1, 9999, -1, 2147483647; unknown IDs | Implemented | Measured unsupported answer 1 (§1.5) |
| 101, 121 | Needs spec | Playlist effects explicitly unresolved (§13.5) |
| 135 | Needs implementation outside this surface | Restart effect measured, but existing app command path has no restart command; not aliased to Quit |
| 242, 501 | Needs spec | Environment/state-dependent values not established |
| 260, 261, 281, 800, 900 | Needs spec | Handle ownership/contents, hidden-state visibility or job-derived semantics open; never return reference-process addresses |
| 100, 120, 129, 130, 199–202, 211–214, 240–249 (except above), 259, 280, 290–296, 334–336, 365, 500, 505–510, 532/533 | Needs spec | Unmeasured/pointer forms (§8, §12); notably 212 title contents are not settled |
| 600/601/603/611–613/618/622/623/628/631; remaining §12.3 IDs | Needs spec | Notifications, unmeasured 5.x or pointer contracts; no guessed behavior |
| `WM_COPYDATA` 100/103; `0x0402`; other `WM_USER+n` | Needs spec | Left to the existing window chain, not conflated with IPC (§4, §13) |
| `WM_COMMAND` | Already handled by command path | Low word selects actions from existing `MAIN_TABLE`/`GLOBAL_TABLE`; uses `App::activate_menu_item`, identical to key/menu dispatch. Unknown IDs pass to winit; no new ID catalog |

Sent IPC setters are deliberately unsupported and cause no mutation: their
return contract is unmeasured. Use **posted** setters and subsequent sent
queries. `WM_COMMAND` accepts both sent and posted delivery. No core/protocol
contract was changed.

## Tests

Pure unit tests cover parameter order/sign conversion, all existing main/global
command mappings, nondefault live values, unknown IDs, measured posted effects
and explicit spec gaps. No window is created by default tests.

`external_control_drives_the_player_window_in_the_test_vm` in `tests/headless.rs`
is **ignored, Windows/VM-only** and refuses outside the test VM. Run it only
through the interactive guest-session runner with
`--ignored:external_control_drives_the_player_window_in_the_test_vm`. The test
process sends actual cross-process messages to the spawned player's HWND,
checks its real class and replies, drives the existing volume/shuffle command
path, and verifies posted pan/shuffle/repeat setters via read-back. The child
uses `CREATE_NO_WINDOW` and is killed/reaped even on assertion failure. This
test has not been run by the implementation lane.

Lane verification (headless, 2026-10-19):

- `cargo fmt --all -- --check`: passed.
- `cargo clippy -p rustamp-app --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p rustamp-app --all-features`: passed, 293 tests passed,
  11 ignored across 16 test executables (plus empty doc-tests). Includes six
  new pure external-control tests; the new real-window test remains ignored.
- `git diff --check`: passed. No GUI/ignored test was run.

## Provenance

Used only `specs/S10-ipc.md` for new behavior, plus Rustamp's existing window,
shortcut, menu, state and plugin-host code for integration/coverage review.
No Winamp source, SDK material, disassembly, research-repository content or
third-party copied code was read/used. No dependencies were added.
