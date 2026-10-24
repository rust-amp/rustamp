# Task card F4 — Display scaling and multiple monitors (M2)

**Goal**

Give the player platform support for display changes: know which monitors
exist and what their work areas are, know each monitor's scale factor, know
when a monitor is added or removed, when the primary monitor changes, and
when a monitor's scale changes — and provide the rule that re-clamps a saved
window position that no longer fits on any monitor onto a visible one. The
window side applies it (seams below); this card owns the platform side.

**Lane** F — platform (`rustamp-platform`).

**Owned paths**
- `crates/rustamp-platform/**` (new `displays.rs`; `lib.rs` re-exports and
  crate doc; a module-doc cross-reference in `dpi.rs`; one `windows` feature
  added to `Cargo.toml`)
- this card, `docs/tasks/F4-platform-displays.md`

No edit outside those paths was made. In particular `crates/rustamp-app`
(including `window.rs`) is untouched: the window-side application is a
follow-up for the lane that owns `window.rs`, and the exact seams are
described below rather than implemented here.

**Allowed sources**
- `docs/PLAN.md` §5 M2 (platform).
- `docs/tasks/BETA-readiness.md` BETA-11 (the gap this card narrows: platform
  integration "still a stub apart from DPI").
- `specs/S05-docking.md` §3.4 (persistence across a monitor change — **open**)
  and §6 (the window keeps its pixel size across monitors; logical-size
  preservation is app-layer/S05 work; the reference's live multi-monitor
  `WM_DPICHANGED` behaviour is **unmeasured**).
- `docs/HANDOFF.md` §5 §10 (the DPI/window-sizing remainder this card picks
  up on the platform side).
- The existing `crates/rustamp-platform` (F1 DPI, F2 single instance, F3
  tray/hotkeys) and the `windows` crate 0.62 API (verified against the
  vendored sources in the local Cargo registry; no Winamp material read).

S13 does not exist as a spec, and no spec settles what the reference player
does with a saved position after a monitor change (S05 §3.4 is open; §6 names
the live transition unmeasured). The clamping and reporting behaviours are
therefore **own choices**, stated as such below; none is promoted from a
guess to a fact.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
- No downloads, no `cargo add`, no new dependencies, nothing outside
  `Cargo.lock`. (One `windows` **feature** was added — `Win32_Graphics_Gdi`
  for `EnumDisplayMonitors` / `GetMonitorInfoW` / `MONITORINFOEXW` — still
  nothing new is downloaded, because `windows` is one crate with feature-
  gated modules; same precedent as F2 and F3.)
- No reference binary run, no window, no audio.

**Scope**
- **In scope**
  - A `rustamp-platform` displays module: `ScreenRect`, `Monitor` (device
    name, bounds, work area, primary flag, effective DPI), `DisplaySnapshot`,
    a pure diff (`display_changes` → added / removed / primary change / scale
    change / geometry change), scale-factor selection
    (`monitor_for_rect` / `monitor_for_point` / `Monitor::scale_factor` /
    `dpi_at`) and the saved-position clamp (`clamp_position` /
    `clamp_saved_position`).
  - The Win32 backend: `EnumDisplayMonitors` + `GetMonitorInfoW` +
    `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`, with a non-Windows stub.
- **Out of scope:** any `window.rs` application (seams below; another
  lane's file), logical-size preservation across monitors (S05 app layer),
  snap rules (S05 §4.1 open), re-rendering on a DPI change (the F1 contract
  keeps physical pixel size), macOS/Linux, docked-neighbour re-flow (the
  existing `DockedStack` arithmetic in `rustamp-ui`).

**Interface contract**
- `rustamp-platform` keeps depending only on `rustamp-core`, `thiserror` and
  `windows` (Win32). It does **not** depend on `winit`, `softbuffer` or
  `rustamp-ui`, and it does not call `App` (F1/F2 contract, kept).
- No `rustamp-core` contract changes: the module is pure data plus Win32
  queries; no new `Command` or config field.
- The window side needs exactly two convenience entries, both degrading
  (never wedging): `clamp_saved_position(x, y, width, height)` at window
  creation, and `DisplaySnapshot::capture()` + `display_changes(last,
  current)` polled from the UI pump.

**Design (and why)**

- **Pure core, thin Win32** (F2 pattern): every decision point is a pure
  function over `DisplaySnapshot`, so all rules are unit-tested headless
  against fake display sets; only `capture()` touches the OS, and the
  in-process Win32 test follows `dpi.rs`'s session-0 carve-out.
- **Monitor identity = the Win32 device name** (`MONITORINFOEXW.szDevice`,
  e.g. `\\.\DISPLAY1`) — the only per-monitor identity the API provides —
  and the diff is keyed by it. Own choice.
- **Visible = intersects a work area**, not the full bounds: a window parked
  entirely under the taskbar is not visible. Own choice.
- **Minimal intervention:** a saved window that still touches any work area
  is left exactly where it was; only a window orphaned by the display change
  moves. Own choice.
- **The clamp target is the primary monitor** (else the first enumerated),
  the window is seated **wholly** inside its work area, coordinates are
  clamped into the work area's range, and a window larger than the work area
  pins to its top-left corner instead of panicking. Own choice; it matches
  the common platform expectation that orphaned windows re-seat on the
  primary work area.
- **Strict capture:** if any monitor's geometry or DPI cannot be read, the
  whole capture fails (`DisplayError::MonitorInfo`) — a partial enumeration
  must not be reported as truth, because it would look like a removal. A
  failed capture degrades: `clamp_saved_position` returns the saved position
  unchanged (F2 "degrade, never wedge").
- **Scale factors are reported, not applied.** S05 §6 (via the F1 contract)
  settles that the window keeps its physical pixel size across monitors, so
  the scale a window "uses" is the effective DPI of the monitor it sits on —
  `monitor_for_rect` picks that monitor (largest work-area intersection, ties
  to the primary, then enumeration order; a window missing every work area
  gets the primary) and `Monitor::scale_factor` is that DPI over 96. It is
  advisory until S05's app-layer logical-size preservation.
- **The primary diff uses the primary flag**, not `primary()`'s
  first-enumerated fallback: a flag appearing or moving is a change
  (`Some((None, Some(b)))` or `Some((Some(a), Some(b)))`), while two
  flagless snapshots are not.

**Own choice vs spec-settled**

| Item | Status |
|---|---|
| The window keeps its physical pixel size across monitors; no re-render on a DPI change | Settled (S05 §6; F1 contract in `dpi.rs`) |
| Per-monitor-V2 awareness before any window | Settled (F1, `dpi.rs:63`) |
| A saved position off every monitor is clamped at all | Own choice (S05 §3.4 open, reference unmeasured) |
| "Visible" = work-area intersection; clamped window wholly inside the target's work area | Own choice |
| Clamp target = primary (else first enumerated); oversized window pins to the origin | Own choice |
| Monitor identity = Win32 device name | Own choice (only stable id the API provides) |
| Scale factor = effective DPI (`MDT_EFFECTIVE_DPI`) over 96, reported not applied | Own choice (the keep-pixel-size side is settled) |
| Strict capture (partial enumeration = error, then degrade) | Own choice (F2 principle) |

**Seams for `window.rs` (follow-up for the lane owning that file; not
implemented here)**

1. **Saved positions at creation.** The three sites that apply a saved
   `WindowLayout` position verbatim — main (`window.rs:1288-1297`),
   equalizer (`window.rs:408-410`), playlist editor (`window.rs:1117-1126`) —
   pass each saved position through
   `rustamp_platform::clamp_saved_position(layout.x, layout.y, outer_width,
   outer_height)`. Pass the window's **outer** physical size (the frame size
   plus the measured chrome, `outer_size_for_client` from F1); the client
   size is an acceptable conservative fallback (a slightly smaller rect is
   less likely to be judged visible, so the clamp errs on the side of
   moving). The function returns the input on any capture failure, so no
   `match` is needed at the call site.
2. **Change reporting in the pump.** From `Windowed::about_to_wait`
   (`window.rs:1484`), throttled (e.g. every ~1 s — `EnumDisplayMonitors` is
   not free and must not run per 16 ms frame), compare
   `DisplaySnapshot::capture()` against the last kept snapshot with
   `display_changes(last, current)`:
   - `removed` non-empty → re-run `clamp_position` (the new snapshot, each
     open window's outer size) for every open window whose position no
     longer intersects a work area, then re-seat the docked neighbours
     (`place_eq_below_main` / `sync_playlist_position` keep the S05 stack
     ordered);
   - `scale_changed` non-empty → record the old and new DPI per monitor; no
     resize (the window keeps its pixel size, S05 §6);
   - `primary_changed` → no action needed: the clamp target follows the new
     primary by construction.

   The alternative is a message-only window receiving `WM_DISPLAYCHANGE`;
   the throttled poll is fewer moving parts, and `winit` already delivers
   the per-window `WM_DPICHANGED` (`ScaleFactorChanged`) that this diff
   complements (it does not name the old/new scale or any monitor).
3. **Scale query (optional).** `dpi_at(x, y)` or
   `snapshot.monitor_for_rect(rect).scale_factor()` for the record.

**Acceptance tests**
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      clean.
- [x] `cargo build --workspace` (offline) green.
- [x] Headless unit tests against a fake display enumeration (compiled by
      `clippy --all-targets` here; **not run on this host** — tests run in
      the Hyper-V VM only): a saved position off every monitor is clamped
      onto the primary (both far corners); a saved position inside or partly
      on a work area is left alone; an oversized window pins to the work
      area origin without panicking; an empty snapshot yields no clamp; a
      clamped position lands wholly inside the target work area (property
      loop over four orphaned positions); a removed monitor is reported and
      a position that lived on it re-seats without panic; an added monitor is
      reported; a primary-monitor change is detected and the clamp target
      follows the new primary; a scale change is reported with old and new
      DPI; a geometry change is reported; an unchanged set reports nothing;
      `monitor_for_rect` prefers the largest work-area intersection, breaks
      ties by the primary and falls back to the primary when every work area
      is missed; `monitor_for_point` and `scale_factor` (96→1.0, 144→1.5,
      192→2.0); the primary falls back to the first enumerated and a
      flagless→flagged transition is still a primary change.
- [x] In-process Win32 test with the `dpi.rs` session-0 carve-out:
      `capture()` returns a non-empty sane set (every monitor carries its
      device name, a positive-size bounds and a work area inside it, and a
      positive DPI; exactly one primary in an interactive session), and a
      typed error is the contract in a non-interactive session.
- [ ] `cargo test --workspace --all-features` green in the test VM (the new
      tests were compiled but not run here).
- [ ] **Needs the VM / a real desktop** (nothing below can be exercised
      headless; the VM carries one virtual monitor, so multi-monitor checks
      need a second display or the virtual display driver's settings):
   1. Hot-plug: adding a monitor reports `added` and moves no window;
      removing one reports `removed` and re-seats any window that lived on
      it onto the primary work area.
   2. A scale change on the virtual monitor (100 % → 150 %) reports
      `scale_changed` with both DPI values, and the window keeps its
      physical pixel size (S05 §6).
   3. A primary change (needs the second display or the driver's primary
      setting) is detected as `primary_changed`.
   4. The `window.rs` seams themselves (a lane owning that file): the clamp
      at the three creation sites, the throttled poll + `display_changes` in
      `about_to_wait`, and the live re-clamp of open windows on a removal.

**Provenance notes**
- No free-source code was taken. The Win32 calls are used through the
  vendored `windows` crate; item names and signatures were verified against
  the local registry sources (API facts, not copied code). No Winamp
  material was read.

**Report back**
Files and line counts; the current-state findings (what the crate did
before this card, with file:line); the design and why; own choice vs
spec-settled; the seams the window must call; the test list and the VM
checklist; fmt/clippy/build results; and the confirmation that no `cargo
test`, no window, no audio and no commit were made on this host.
