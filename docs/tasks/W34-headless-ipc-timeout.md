# W34 — Player-window IPC startup readiness and strict guest timeout

## Scope and provenance

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave134-headless`.
- Owned: `crates/rustamp-app/**`, `crates/rustamp-ipc/**`,
  `crates/rustamp-platform/**` and this card. Only app files/card changed.
- W32 was read first. Other sources: required repository handoff/rules,
  existing clean player/window/tray/test code, `specs/S10-ipc.md` channel and
  version facts, installed MIT-licensed winit 0.30.13 source (class callback and
  native window setup). No reference binaries were run. No Winamp source, SDK,
  disassembly or research repository was read; no third-party code was copied.
- No downloads, dependencies, core/wire/classic IPC contract changes, commits
  or git writes. Build/test output and temporary files stay under this tree's
  `target/w34` (Cargo artifacts under `target`). No GUI test ran on the host.

## What the old barrier did, and what the evidence actually proves

W32 bounded retries of IPC 0 until the exact version `0x2905`. It fixed the
first failed send escaping the old retry loop, but still used a **reply** as a
startup completion signal. Finding the visible main HWND was even earlier:
`Windowed::resumed` created it before surface/tray initialization and installed
the answering subclass only afterward. Before installation a sent message can
reach winit's normal class procedure; there is no replay into a later subclass.

The class name is assigned through winit's `with_class_name`; winit registers
its own `public_window_callback`. That class does not implement our S10 reply.
Only `ExternalControl::attach` installs the player-local subclass. IPC 0
decodes to a constant reply; it does not visit the plugin host, audio engine,
an app pump, or a remote transport.

**Do not infer that late installation can follow a successful exact version
barrier.** The version is evidence that the installed handler already ran.
Normal operation does not remove it: removal is on guard drop/window
destruction, and the event loop drops the guard on exit. A missing subclass
alone cannot explain a later failed send after that successful barrier.
However, sent Win32 messages can run reentrantly during native startup calls:
one reply proves neither completion of the remaining `resumed` work nor the
first full app/presentation tick. A subsequent send can still time out while
that UI thread is busy. This is a possible scheduling path, **not a measured
identification of the guest's blocking operation**.

The callback also used to acquire the mutable bridge before constant replies;
an overlapping borrow returned unsupported (1), not the version. That could
lose a readiness reply, but is **not** a `SendMessageTimeoutW` failure (API
result 0), and is not claimed as the cause of the reported guest timeout.

The reported `assert_ne!` panic text/line is also not the Result-returning
helper found in this worktree's W32 baseline. A new guest receipt must identify
the staged binary/revision and include the saved child logs before attributing
that old assertion to the current barrier.

## Changes and preserved guarantees

1. Create the main window hidden; install the answering subclass immediately,
   before softbuffer/tray work or showing it. Installation failure exits window
   startup instead of logging and exposing a visible, unanswerable player.
2. The lifetime-owned handler publishes the private HWND property
   `Rustamp.ExternalControl.Ready = 1` only at the end of the first full
   `about_to_wait` tick: after state publication, presentation, window syncing
   and modal-request drains. Drop and `WM_NCDESTROY` remove the marker.
3. The guest startup barrier polls that property using `GetPropW`, which reads
   window-manager data without sending into the player's thread. It retains
   the existing 10 s readiness budget and child-exit check. It waits for an
   **installed handler plus completed startup tick**, not a reentrant version
   response. The first version query and every subsequent IPC send retain
   exactly 1000 ms, with no failed-send retries. Wrong replies still fail.
4. Constant replies bypass bridge borrowing. Supported commands reaching the
   installed handler before readiness still queue for the normal app drain;
   no readiness gate discards them. A hidden HWND probed before attachment is
   still not a supported ready endpoint; messages already handled by winit
   cannot retrospectively be recovered.
5. Startup logs distinguish hidden-window handler installation, completion of
   `resumed`, and first full tick/readiness. Existing panic cleanup prints child
   stdout/stderr and kills/reaps the child.

A player stuck before readiness fails the bounded barrier; a dead player fails
the child check; a missing handler cannot publish readiness; a player stuck
after readiness fails the very next strict send. No timeout, expected reply,
setter non-mutation assertion or posted-command readback was relaxed.

## Host verification (2026-10-20)

Each exit code was captured immediately after its Cargo invocation. Offline
Cargo and a local target/temp directory prevented downloads/external writes.

| Gate | Exit | Receipt |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | `target/w34/fmt.log` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | `target/w34/clippy.log` |
| `cargo test --workspace --all-features` | 0 | `target/w34/test.log` |

Full suite: **2498 passed, 0 failed, 47 ignored**, 142 result blocks including
doc tests. Three new headless regressions pass: constant version delivery under
a held bridge borrow, early handler query/command retention, and a structural
guard on hidden-create/install/show ordering and full-tick readiness placement.
The structural guard is source-order evidence, not a substitute for Win32
runtime scheduling. `git diff --check`: 0. Read-only canonical-tree check:
`git -C D:\code\Rustamp status --short`: exit 0, empty output.

## What only the interactive guest can confirm

Run `external_control_drives_the_player_window_in_the_test_vm` against newly
built/staged artifacts through the existing interactive-session runner. The
real cross-process property visibility, initialization timing and all strict
IPC/command assertions remain **unrun here**. Also rerun the existing main/EQ/
playlist window probes because the main window now starts hidden.

If it still fails, preserve the exact staged revision and the printed child
logs. Failure before the marker versus failure of the first/later strict send
now distinguishes startup incompletion from post-readiness unresponsiveness.
The specific guest wait/stack causing a remaining timeout still needs guest
runtime evidence; this lane does not claim that a successful host build/test
proves the reported guest failure fixed.
