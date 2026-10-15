# Task card F1 — Per-monitor DPI awareness and correct window sizing (M1)

**Goal**
Make the main-window example a correctly sized native window on any system DPI:
the process is per-monitor-V2 DPI aware, the client area is exactly the layout
size in physical pixels (275×116, 550×232 when double-sized), and double-size
resizes the OS window instead of only the softbuffer surface. No docking, no
menus, no new behaviour — only correct sizing.

**Lane** F — platform (`rustamp-platform`, presenter wiring).

**Owned paths**
- `crates/rustamp-platform/**`
- `crates/rustamp-ui/src/presenter.rs`
- `crates/rustamp-ui/examples/**`
- Crate-level `Cargo.toml` files under the owned paths only (you may add
  `windows.workspace = true`; it is already in `Cargo.lock`, so nothing new is
  downloaded). Do **not** edit the root `Cargo.toml`, `rustamp-core`,
  `rustamp-proto`, `rustamp-skin`, `rustamp-gfx`, `rustamp-audio`, `xtask`,
  `docs/**` or `THIRD_PARTY.md`. If something outside these paths must change,
  report it as a finding instead of editing it.

**Allowed sources**
- `specs/S01-window-geometry.md` (audited): the 275×116 layout size.
- `docs/HANDOFF.md` §5 open points 10 and 11: the DPI facts (sandbox at system
  DPI 144; a DPI-unaware process sees 183×77; landmark check
  `GetClientRect == 275×116`).
- `crates/rustamp-ui/src/presenter.rs` (what D1 built), `crates/rustamp-platform`
  (the stub and its doc comment), public `winit` documentation.
- The `windows` crate 0.62 API (already vendored) for the DPI-awareness call.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything: no `cargo add`, no new crate versions, no web
  fetching. If the work needs a crate version that is not in `Cargo.lock`,
  stop and report it.
- Do not run a reference binary.

**Scope**
- **In scope:**
  - A `rustamp-platform` init function (suggested: `init_dpi_awareness()`) that
    makes the process per-monitor-V2 DPI aware at startup, called before the
    `winit` event loop. It must be idempotent and return a typed error, never
    panic.
  - The presenter sizes the OS window so the **client area** is exactly the
    layout size in physical pixels (275×116 normal, 550×232 double-size),
    accounting for window chrome. Toggling double-size resizes the OS window
    and re-creates/sizes the softbuffer surface to match.
  - Behaviour on a DPI change while running (`WM_DPICHANGED`): at minimum the
    client area must not end up smaller than the layout; document what happens.
  - Headless-testable size math: the chrome-adjustment computation lives in a
    function that takes numbers, not a window, so tests cover it without
    opening anything.
- **Out of scope:** docking/snapping (S05), menus, windowshade behaviour, single
  instance, tray, global hotkeys, SMTC, file associations, the
  `Winamp v1.x`-compatible window. Do not start any of them.
- Never promote a gap to a fact. If `winit` does not expose something you need
  (for example exact chrome insets before the window is shown), say so and pick
  the smallest documented workaround.

**Interface contract**
- `rustamp-platform` keeps depending only on `rustamp-core` plus `windows`
  (Win32); it must not depend on `winit`, `softbuffer` or `rustamp-ui`.
- The presenter keeps owning the clock and the `MainWindow` machine; this card
  adds sizing only, no behaviour.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for the
  chrome-adjustment math (275×116 and 550×232 targets, asymmetric chrome, zero
  chrome) and the init function's idempotence contract.
- [ ] `cargo test -p rustamp-skin` still green.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch; `cargo xtask ring-cross` still passes.
- [ ] The example opens with a client area of exactly 275×116 physical pixels on
      the sandbox machine (system DPI 144), verified with `GetClientRect` on its
      own window; toggling double-size gives exactly 550×232. State in the report
      how you measured. No input to the user's desktop: measure your own window
      only, never move the cursor, close the window afterwards.

**Provenance notes**
- No free-source code is expected. If you take anything from anywhere, it needs
  attribution — but you may not edit `THIRD_PARTY.md`, so report it as a finding.

**Report back**
Files changed; how DPI awareness is established and where it is called; how the
client-area sizing works and what happens on a DPI change; the `GetClientRect`
readings before and after (normal and double-size, at DPI 144); the new test
list and results; anything you could not verify; and questions that belong to
S05 or the app layer.
