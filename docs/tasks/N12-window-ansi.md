# N12 — "window titles and class are one character long": the truncation is the smoke reader's, not the player's

**Date:** 2026-10-18. **Status:** investigation complete, test added, VM checks
pending (this lane runs no test and no VM).

**Worktree / write boundary:** `wave81-window-ansi` worktree only. Changed:
`crates/rustamp-app/tests/child_windows/mod.rs`,
`crates/rustamp-app/tests/headless.rs`,
`crates/rustamp-app/tests/playlist_window.rs`, and this card
(`docs/tasks/N12-window-ansi.md`). Audited and **unchanged**:
`crates/rustamp-platform/**` (every Win32 string call is wide-correct) and
`crates/rustamp-plughost/**` (same — the mistake is not there, so per this
card's ownership it is not touched). No runner script, broker, or other crate
touched; no download, no new dependency, no git write, no window, no audio,
no test run, no VM.

## 1. The evidence and what it actually names

The seventh packaged-player smoke run failed the window step and the X5 dump
(`docs/tasks/X5-smoke-window.md`) reported the player's visible owned
windows as:

```text
pid=9416 title="R" class="W";  pid=9416 title="R" class="W";  pid=9416 title="" class="W"
```

Those three windows are exactly the player's three **visible** top-level
windows on a fresh config (`RUSTAMP_CONFIG` points at a non-existent file, so
the defaults apply; `eq_open` defaults to false and `playlist_open` defaults
to **true** — `crates/rustamp-app/src/app.rs:236-237`):

| # | Window | Real title | Real class | Read as |
|---|---|---|---|---|
| 1 | main window (`window.rs:1302-1305`) | `Rustamp` | `Window Class` | `R` / `W` |
| 2 | playlist editor (`window.rs:1113-1116`) | `Rustamp Playlist` | `Window Class` | `R` / `W` |
| 3 | winit's thread event-target window | *(none — created with a null title)* | `Winit Thread Event Target` | `""` / `W` |

Window 3 is created by `winit` 0.30.13 itself (`event_loop.rs:904-963` in the
local cargo cache: class `"Winit Thread Event Target"` at :904-905,
`RegisterClassExW` at :928, `CreateWindowExW` with a **null** title at
:939-942, then forced `WS_VISIBLE | WS_POPUP` at :960 so `IsWindowVisible`
reports true while a layered style keeps it off the screen). Its class name
exists **only inside winit** — no Rustamp code registers or names it. That
is the decisive clue: a one-character class cannot come from the player's
creation path (nothing registers a one-character class); it can only come
from a **reader** that truncates. The player's windows are intact.

## 2. Where the truncation happens (file:line, API variant, string type)

`docs/notes/vm-broker/guest/run-smoke.ps1:198-199`:

```powershell
[DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
[DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
```

The **wide** API variants (`GetWindowTextW`, `GetClassNameW`) are called, but
the `StringBuilder` out-parameter is marshaled as an **ANSI byte buffer**:
`DllImport`'s default `CharSet` is `Ansi`, and neither declaration carries
`CharSet = CharSet.Unicode`. The wide API writes UTF-16 into that byte
buffer; on return the marshaler decodes the buffer back through the ANSI
code page, which stops at the first NUL byte. Hence `Rustamp` → `R`,
`Window Class` → `W`, `Winit Thread Event Target` → `W`, and an empty title
→ `""` — precisely the dump. The helpers built on those declarations are
`ClassName` (`run-smoke.ps1:213-218`) and `Title` (:219-223); the window
wait uses `Title` (:270-278, called at :271) and the timeout dump uses both
(:303-316, called at :309-310). That is why the smoke could never find the
documented `Rustamp` title while the player was alive with its window open,
and why the earlier runs failed the same way. (The pattern predates X5: the
pre-X5 wait read the title through the same StringBuilder shape; X5 kept it
and only widened the buffer.)

A second, latent hazard in the same two lines: with `CharSet.Ansi`,
`StringBuilder(512)` allocates 512 **bytes**, while `n` is passed as
`s.Capacity` = 512 **characters** — `GetWindowTextW` may write up to 512
`wchar_t` (1024 bytes) into that buffer, so a title of ≥ 256 characters
would overrun the marshaled buffer. The fix below makes capacity and `n`
agree in characters (512 chars = 1024 bytes).

## 3. The player is intact: every window creation site, API variant, string type

| Window | Site | Title passed | Class | API variant actually used | Verdict |
|---|---|---|---|---|---|
| main | `crates/rustamp-app/src/window.rs:1302-1305` (`.with_title("Rustamp")`), title updates at `window.rs:704-713` (`set_title`, :710) | Rust `&str` → winit `encode_wide` (`winit 0.30.13 platform_impl/windows/window.rs:1307`, `:115`) | winit's `Window Class` (`winit mod.rs:51`, registered `window.rs:1309,1421`) | `CreateWindowExW` (`winit window.rs:1367`) / `SetWindowTextW`; `RegisterClassW` | wide API + wide buffer — **correct** |
| playlist | `window.rs:1113-1116` (`.with_title("Rustamp Playlist")`) | `&str` → winit | `Window Class` | `CreateWindowExW` | **correct** |
| equalizer | `window.rs:397-400` (`.with_title("Rustamp Equalizer")`) | `&str` → winit | `Window Class` | `CreateWindowExW` | **correct** |
| menu popup | `window.rs:907-916` (`menu_window_attributes`, `.with_title("Rustamp Menu")` at :909) | `&str` → winit | `Window Class` | `CreateWindowExW` | **correct** |
| dialog | `window.rs:741-744` (`.with_title(kind.title())`) | `&str` → winit | `Window Class` | `CreateWindowExW` | **correct** |
| plughost host window (child process) | `crates/rustamp-plughost/src/host_window.rs:325-343` (`RegisterClassW`, `lpszClassName: PCWSTR` at :333 over `host_window_class_wide()` :49-53 = `"Winamp v1.x"` as NUL-terminated UTF-16) and `:345-379` (`CreateWindowExW` at :360 with `PCWSTR` class :362 and `PCWSTR` title `"Rustamp plughost"` :349-352, :363) | NUL-terminated `Vec<u16>` | `"Winamp v1.x"` (ABI-fixed, S09) | `RegisterClassW` / `CreateWindowExW` | wide API + wide buffer — **correct** |

The app crate is `#![forbid(unsafe_code)]` (`crates/rustamp-app/src/lib.rs:47`),
so it can hand strings to Win32 only through `winit` (wide, above) and `std`
(`std::process::Command` in `supervise.rs:453-459` → `CreateProcessW`;
file I/O → `CreateFileW`) — all wide-correct.

## 4. What else could be affected: every other string handed to a Win32 call

| String | Site | API | Pairing | Verdict |
|---|---|---|---|---|
| tray tooltip `"Rustamp"` | `crates/rustamp-platform/src/tray.rs:180-186` (`encode_utf16` into `NOTIFYICONDATAW.szTip`) → `Shell_NotifyIconW` (:200) | wide | wide/wide | correct |
| single-instance mutex name | `single_instance.rs:412-418` (`to_wide` :610-612 → `PCWSTR`) → `CreateMutexW` | wide | wide/wide | correct |
| single-instance pipe name | `single_instance.rs:559-577` (`to_wide` → `PCWSTR`) → `CreateNamedPipeW`; the forwarding side opens it via `std::fs::OpenOptions` (:483, :513 → `CreateFileW`) | wide | wide/wide | correct |
| global hotkeys | `hotkeys.rs:418-429` `RegisterHotKey`/`UnregisterHotKey` | — | no strings | correct |
| DPI awareness | `dpi.rs:78-109` | — | no strings | correct |
| monitor device name | `displays.rs:446` `GetMonitorInfoW`, decoded `from_utf16_lossy` (:523) | wide | wide/wide | correct |
| host window class + title | `host_window.rs:325-343, 345-379` (see §3) | `RegisterClassW`/`CreateWindowExW` | wide/wide | correct |
| plugin DLL path | `crates/rustamp-plughost/src/loader.rs:705-726` (`wide_path` :786-797, `encode_wide` + NUL) → `AddDllDirectory`/`LoadLibraryExW` | wide | wide/wide | correct |
| plugin export names | `loader.rs:752-768`: `GetProcAddress` is **ANSI-only** and is declared `lpProcName: *const u8` (:73-76), given NUL-terminated `&[u8]` (`rustamp-plugin-abi/src/common.rs:102-110`) | ANSI | ANSI/ANSI | correct pairing |
| IPC shared-memory/event names (audited, not owned) | `crates/rustamp-ipc/src/shm/wake.rs:96-117`, `region.rs:157,189,260` (`encode_utf16` + NUL → `PCWSTR`) → `CreateEventW`/`OpenEventW`/`CreateFileMappingW`/`OpenFileMappingW` | wide | wide/wide | correct |
| repo's own window readers (the correct pattern) | `crates/rustamp-app/tests/child_windows/mod.rs:41-42,72-91` (`GetWindowTextW`/`GetClassNameW` into `[u16; 256]`, `from_utf16_lossy`), `xtask/src/capture.rs:42-45,58-75` (`GetClassNameW`/`GetWindowTextW` + `[u16; N]`) | wide | wide/wide | correct |
| guest script `SmokeWin` | `docs/notes/vm-broker/guest/run-smoke.ps1:198-199` | `GetWindowTextW`/`GetClassNameW` (wide) | **wide API + ANSI-marshaled `StringBuilder`** | **WRONG — the truncation** |

No menu label, tooltip, registered-message name, mutex or pipe name is
handed to a Win32 call anywhere in the Rust code except the rows above, and
every one of them pairs correctly. The single wide/ANSI mismatch in the
whole packaged-player + smoke toolchain is the guest script's two
declarations.

## 5. The rule (a rule, not a spot repair)

**An A/W pair must match on both sides of the boundary.** A `…W` API takes
a NUL-terminated UTF-16 buffer — in Rust a `Vec<u16>`/`[u16; N]` behind
`PCWSTR` (see `single_instance.rs:610-612`, `host_window.rs:49-53`,
`child_windows/mod.rs`, `xtask/src/capture.rs`); in .NET a `CharSet.Unicode`
`DllImport` (or a wide `char[]` buffer). An `…A` or ANSI-only API (e.g.
`GetProcAddress`) takes bytes. Handing one variant the other's buffer
truncates every string to its first character, because the mismatched reader
stops at the first zero byte. The Rust codebase already follows this rule at
every site; the one violation is the guest script's `SmokeWin` helpers.

## 6. The fix

- **Crates: nothing to fix.** Every window-creation site and every other
  Win32 string call already pairs correctly (§3, §4). Fabricating a
  "fix" here would be a spot repair of a non-bug.
- **Smoke lane (NOT applied here — the runner is not this lane's path):**
  add `CharSet = CharSet.Unicode` to both declarations at
  `docs/notes/vm-broker/guest/run-smoke.ps1:198-199`, e.g.
  `[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(...)`
  (equivalently: read into a wide `char[]` buffer). That single change makes
  the window wait see the real `Rustamp` title and the dump report full
  titles/classes; it also removes the ≥256-character buffer-overrun hazard
  (§2). **No check is weakened**: the wait's documented title rule
  (`Rustamp` exactly, or the `Rustamp - ` prefix form) is unchanged — after
  the fix it is matched against the real title instead of a truncated one.
- The smoke lane should also give its own reader a round-trip assertion
  (e.g. the dump must show a full multi-character title for a window it
  knows exists), because no crate test can cover a guest script. That is
  the smoke lane's change, not this one's.

## 7. The test (this class of bug, not this instance)

**Pointed at (already existing, VM-only):** `find_window`
(`crates/rustamp-app/tests/child_windows/mod.rs:107-132`) matches the exact
multi-character title `"Rustamp"` (used at `tests/headless.rs:307` and
`tests/playlist_window.rs:115-116`) through the wide reader — a truncated
title on the player's writing side, or a truncating crate-side reader,
fails there (an exact `!=` never matches `"R"`).

**Added this lane (the gap the evidence exposed — the class was never
read, and the title match was implicit):**

- `crates/rustamp-app/tests/child_windows/mod.rs`: `GetClassNameW`
  declaration (:42), `window_title` made `pub` (:72-78), and a new
  `pub fn window_class` (:85-91) reading the class with the wide API into
  a `[u16; 256]` buffer — the A/W-correct pattern, mirroring
  `window_title`.
- `crates/rustamp-app/tests/headless.rs` (`the_windowed_player_opens_a_window_inside_the_test_vm`):
  asserts the main window's title round-trips exactly `"Rustamp"`
  (:315-319) and its class round-trips exactly `"Window Class"` (:320-326)
  (winit 0.30's registered class; Cargo.lock pins winit 0.30.13 — if a
  winit upgrade renames it, the failure names the rename; the point is
  that the class round-trips in full, not that it is one character).
- `crates/rustamp-app/tests/playlist_window.rs` (`the_playlist_window_opens_docked_below_the_main_window`):
  the same title round-trip for the main (`"Rustamp"`, :122-126) and
  playlist (`"Rustamp Playlist"`, :127-131) windows, plus the class
  round-trip for both (:132-145).

**These need a real window** (they spawn the real player and read its OS
windows), so they are **VM checks**: `#[ignore]`d, gated on
`rustamp_app::window::require_test_vm()`, exactly the shape of the tests
extended. This lane does **not** run them (no `cargo test`, no VM, no
window on the host); the orchestrator's suite run in the test VM is the
check, and the assertions compile and clippy-check on the host.

Scope note: the equalizer, menu and dialog windows share winit's title path
(`with_title`) and class with the main window, so the main-window
round-trip covers the mechanism for all of them. The in-process probes
(`tests/eq_window.rs`, `tests/main_menu.rs`) cannot read the OS class
without a new dependency (`raw-window-handle` for the raw `HWND`), which
this lane must not add. The plughost's host window (class `Winamp v1.x`,
title `Rustamp plughost`) is the only raw-Win32 window in the owned crates
and is wide-correct; a round-trip test for it would belong to the plughost
lane, which owns that crate only if the mistake is there — it is not.

## 8. Verification and boundary receipt

- `cargo fmt --all -- --check` — **clean** (no diff).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  **0 warnings** (the new test code compiles on this Windows host, so the
  `#[cfg(windows)]` blocks are type-checked and linted here).
- `cargo build --workspace` — **succeeds**.
- **No test was run** (`cargo test` is VM-only; only the orchestrator runs
  it), **no window was opened**, no VM, no player, no broker, no runner
  script was run or touched; no download, no install, no new dependency, no
  commit or git write.
- `git status --short` (worktree) — exactly the four files above
  (three modified test files, one new card); nothing outside the owned
  paths. The main checkout (`D:\code\Rustamp`) was never written to.
- Files and line counts (HEAD → this lane):
  - `crates/rustamp-app/tests/child_windows/mod.rs`: 135 → 160 (+29/−4)
  - `crates/rustamp-app/tests/headless.rs`: 412 → 437 (+25/−0)
  - `crates/rustamp-app/tests/playlist_window.rs`: 141 → 173 (+33/−1)
  - `docs/tasks/N12-window-ansi.md`: new, 191 lines
- Provenance: clean-tree sources only — the repo's own crates, the smoke
  guest script and cards (`docs/tasks/X5-smoke-window.md`,
  `docs/tasks/BETA-smoke.md`, `docs/tasks/X6-smoke-copyin.md`), and the
  `winit` 0.30.13 sources in the local cargo registry cache (a
  MIT/Apache-2.0 third-party dependency), read only to confirm the
  registered class name and the event-target window's creation. No Winamp
  source, SDK header, disassembly or dirty-room material was read or
  touched.
