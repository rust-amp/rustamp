# Task card N2 — Fix the gaps the app wiring exposed (M1)

**Goal** Lane N1 wired the app and reported ten gaps. Fix the ones that are
**defects or small API holes**, and report the rest. No contract change: anything
that would alter a wire type is a finding, not an edit.

**Lane** N — application. **Owned paths** `crates/rustamp-audio/**`,
`crates/rustamp-ui/**`. Do **not** touch `rustamp-core`, `rustamp-proto`,
`rustamp-ipc`, `rustamp-app`, the root `Cargo.toml`, `xtask` or `docs/**` —
report those needs instead.

**Allowed sources** `docs/tasks/N1-app-wiring.md` and N1's report (the ten
findings), `crates/rustamp-app/src/**` (read-only, to see how the app works
around each gap), `crates/rustamp-audio/src/**`, `crates/rustamp-ui/src/**`,
`specs/S02-main-window-behaviour.md`, `specs/S03-shortcuts.md` (audited).

**Fix, in this order (all inside the owned paths):**
1. **`Engine::new` reports `volume: 255` while it initialises the gain from
   `EngineConfig`** — a real bug: `PlayerState.volume` disagrees with what is
   applied. Make the reported state match the applied gain.
2. **`Engine::finish_run` re-bases the timeline to 0**, so a caller cannot report
   how much played. Keep the position meaningful after a completed run.
3. **`MainWindow` has no `set_focused`** — the app clones the state to render the
   focus variant. Add the setter (a UI-local API, not a contract change).
4. **`rustamp_ui::presenter::map_key` is private**, so the app duplicates ~60
   lines of the winit key mapping. Make it public with docs; keep the mapping in
   the presenter.
5. **The R8 tidy-up:** `rustamp-ui/src/shortcuts.rs` still carries a comment
   saying the RFC 0005 payload enums are unreachable through the contract crate
   (RFC 0008 fixed that), and `behaviour.rs` uses the fully qualified
   `rustamp_proto` path. Switch both to `rustamp_core::command::…` and delete the
   stale comment.
**Report, do not fix:** the `Event::TrackChanged.id` gap (needs the engine's
playlist model and is RFC-worthy), the presenter's missing driver hook, the
missing window-control surface (S05), config I/O placement, and the unimplemented
dialogs.

**Hard rules** clean room (no Winamp source/SDK/disassembly/binary, no
`rustamp-research`/`refbin`); no downloads; no git write commands; **run no
tests** — the orchestrator runs the suite in the Hyper-V test VM, because no test
may run on the host; no window may be opened by anything you do.

**Acceptance** `cargo fmt --all -- --check` and `cargo clippy --workspace
--all-targets --all-features -- -D warnings` clean; new unit tests where they fit
(the orchestrator runs them).

**Report** what you fixed and how each is proven by a test; the findings you left;
the fmt/clippy results; confirmation that you ran no test, no window and no git
write.
