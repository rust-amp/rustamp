# Task card F2 — Single instance and command-line forwarding (M2)

**Goal**
Give the player its first real platform behaviour: a second launch of
`rustamp.exe` must **not** open a second player. Instead it hands its command
line to the running player and exits — the running player enqueues and plays the
named file, applies the flags that still make sense, and reports the flags that
were fixed at start-up.

**Lane** F — platform (`rustamp-platform` plus the app start-up wiring).

**Owned paths**
- `crates/rustamp-platform/**`
- `crates/rustamp-app/src/main.rs` (start-up wiring only)
- this card, `docs/tasks/F2-single-instance.md`

One **additive** edit outside those paths was required and is flagged as a
finding: `crates/rustamp-app/src/window.rs` (the pump seam that drains the
hand-off queue). The platform crate cannot reach a running `App`, so the UI
thread must drain the queue somewhere; `about_to_wait` is that seam (four lines).
`window.rs` is not claimed by another lane.

**Allowed sources**
- `docs/PLAN.md` §3 S13 (single instance and command-line switches) and §5 M2.
- `docs/tasks/BETA-readiness.md` BETA-11 (the gap this card closes).
- `specs/S02-main-window-behaviour.md` §1: "With a file given on the command
  line the program starts playing it."
- The existing `crates/rustamp-platform` (F1 DPI) and the app's `cli`/`config`
  modules.
- The vendored `windows` crate 0.62 API for the Win32 calls.

S13 does not exist as a spec. The behaviours it would have fixed are therefore
**own choices**, stated as such below; none is promoted from a guess to a fact.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
- No downloads, no `cargo add`, no new crate versions; nothing outside
  `Cargo.lock`. (The `windows` crate was already a dependency of
  `rustamp-platform`; only `windows` **features** were added — still nothing new
  is downloaded, because `windows` is one crate with feature-gated modules.)
- No reference binary run.

**Scope**
- **In scope**
  - A `rustamp-platform` single-instance module: claim the lock, listen for a
    forwarded command line, and forward it from a second launch.
  - Start-up wiring in `crates/rustamp-app/src/main.rs`: a windowed launch takes
    the lock; a second launch forwards and exits.
  - The UI thread applies a decoded hand-off through the normal command path.
  - The stale-instance requirement, by construction: a lock that a dead process
    cannot leave behind.
- **Out of scope:** media keys/SMTC, tray, taskbar thumb buttons, jump lists,
  file associations, the external-control window, hotkeys, drag-and-drop.

**Interface contract**
- `rustamp-platform` keeps depending only on `rustamp-core`, `thiserror` and
  `windows` (Win32). It does **not** depend on `winit`, `softbuffer` or
  `rustamp-ui`, and it does not call `App`.
- The hand-off is queued; the app drains it with
  `rustamp_platform::single_instance::take_pending()` and turns it into
  `Command`s with `ForwardRequest::commands()`.
- No `rustamp-core` contract changes: the forwarded effects are existing
  `Command::SetVolume` and `Command::AddItems { items, at: None, play: true }`.

**Design (own choices)**

- **Lock = Windows named mutex** `Local\Rustamp.SingleInstance.<key>`. The
  kernel object is owned by the creating process and released by the kernel when
  that process dies, whatever kills it. There is therefore no lock file to clean
  up and no "held but dead" state to distinguish: **a dead instance never blocks
  the next start.** (A named-pipe or lockfile scheme needs a liveness probe and
  can go stale; the mutex cannot.)
- **Channel = Windows named pipe** `\\.\pipe\rustamp-single-instance-<key>`.
  The first instance listens for its whole life; a second instance connects,
  writes one length-framed request and exits.
- **Instance key = FNV-1a of the lowered resolved config path**
  (`rustamp_app::config::default_path()`). Two normal launches share one
  instance; tests that point `RUSTAMP_CONFIG` at their own temp file never
  collide with each other or with a running player.
- **`--null` never claims the lock.** It is the headless test mode; a test must
  be able to run beside a normal player. `--help` exits before any of this.
- **Hand-off content:** the file (enqueue and play) and the volume. Flags that
  were fixed at the first instance's start-up are **refused with a message**,
  printed by the sender: `--no-audio` (output fixed at start-up), `--skin`
  (skin chosen at start-up), `--seconds` (a start-up test aid).
- **Wire format** (hand-rolled, no new dependency): magic `RSF1`, protocol
  `u16`, an optional file (`u32` length + UTF-8), an optional volume (`u8`), and
  a refusal list. `MAX_FRAME_BYTES = 256 KiB` bounds a corrupt or hostile
  client's allocation. On the pipe the frame carries a `u32`-LE length prefix.
- **Degrade, never wedge:** a mutex that cannot be created at all (a rare access
  error) degrades to "start as the first instance without a mutex"; a listener
  thread that cannot be spawned only makes forwarding unreachable, it does not
  fail start-up.

**Acceptance tests**
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      clean.
- [x] `cargo build --workspace` (offline) green.
- [x] Headless unit tests for the pure decision points: `startup_action`
      (free -> first, held -> forward), `instance_key` (stable, path-derived,
      case-folded, `None` fallback), and the hand-off wire format
      (round-trips, bad magic, wrong protocol, every truncation, oversize,
      invalid UTF-8).
- [x] Headless tests for `ForwardRequest::commands()` mapping (volume before
      file; volume-only; empty).
- [x] In-process Win32 tests: a second `claim` reports `AlreadyRunning`; dropping
      the guard releases the lock and a fresh `claim` wins again; a `forward`
      round-trips through the real pipe into `take_pending`; forwarding to an
      unowned key is `Unreachable`.
- [ ] `cargo test --workspace --all-features` green in the test VM (the crate's
      tests were compiled by `clippy --all-targets` here, but not run: tests run
      in the Hyper-V VM only).
- [ ] **Needs the VM / two processes:** a genuine two-process hand-off
      (`rustamp.exe` #1 windowed, `rustamp.exe` #2 with a file) and a real
      crash-kill of #1 followed by a new start. Neither can be exercised in one
      process; the mutex's kernel-release property is why the crash case is
      expected to be free-lock at the next start, but that is a VM check.

**Provenance notes**
- No free-source code was taken. The win32 calls are used through the vendored
  `windows` crate; no Winamp material was read.

**Report back**
Files and line counts; the design and why; own-choice vs spec-settled; the test
list and what still needs the VM; the stale-instance story; the
`window.rs` boundary note; fmt/clippy/build results.
