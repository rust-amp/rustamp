# Task card M3h — Fix the CODEX review findings, in severity order

**Background** An independent review (OpenAI CODEX, review-only, no code touched)
returned verdict: changes required before treating the M3 host as safe and
compatible. 13 findings, all verified by the orchestrator on samples before
being recorded here. Fix them in severity order, one lane per crate-touching
group (two lanes must never share a working directory).

**Allowed sources** `specs/S10-ipc.md` (measured table — every value must trace
to it), `specs/S09-plugin-abi.md` (audited ABI), `docs/rfc/0006-out-of-process-hosting.md`,
`docs/rfc/0009-ipc-version-constant.md`. **Clean room:** never read Winamp
source, SDK headers, disassembly, the 2.95 binary, `rustamp-research` or
`rustamp-refbin`.

## Group A — soundness bugs (first lane; all in `rustamp-plughost`)
1. **Window state can outlive its allocation** (`host_window.rs:151`, `wnd_proc`
   ~:297): `create(&mut IpcSurface)` retains the pointer in `GWLP_USERDATA` but
   the window neither owns nor borrows the surface; safe callers can move/drop
   it while the window lives, and `wnd_proc` rebuilds `&mut` from the pointer
   (use-after-free through a safe API, plus unexamined reentrancy). Fix: stable
   owned storage with interior mutability, lifetime and UI-thread access
   enforced through the API.
2. **IPC 800 returns a foreign address** (`ipc.rs:888`): hard-coded `4407808`
   identifies no Rustamp-owned object; a plugin could jump into unmapped
   memory. S10 records it as shape-only. Fix: never expose it; use the
   unsupported convention until an audited contract permits a real
   host-owned implementation.

## Group B — evidence contradictions (needs spec decisions first)
3. Mutating IPCs (102/135) report effects they do not cause; setters change
   only local surface until the next snapshot overwrites them.
4. Unmeasured replies asserted as established (sent volume/pan returns, `-666`
   on shuffle/repeat, 126 raw Hz pinned to 44100, 242 always available).
5. Window/menu getters (260/281) return zeros; S10 measured nonzero handles.
   → Each needs an S10/spec decision or an explicit "incomplete" label before
   code changes; report them as findings if the decision is missing.

## Group C — robustness gaps (later lanes)
6. Pong on the reader thread: a hung UI looks healthy; no parent-death
   termination independent of the UI.
7. `WM_QUIT` can strand `join` on a blocked stdin reader; distinguish exit
   reasons, unblock I/O before joining.
8. Hello is not a barrier: attach/publish before any greeting; require a
   compatible Hello with a deadline.
9. Unbounded inbound queues + drain-until-empty on both sides; bound them,
   budget per-pump work, coalesce snapshots.
10. E2E and real-spawn tests rely on `#[ignore]` alone; require the VM
    predicate before either spawn (as the window test does).
11. Binary discovery can select an x64 `rustamp-plughost32.exe`; enforce i686
    and validate architecture.
12. `stop()` does not stick: next supervise restarts; add stopped-vs-detached
    state and a persistent-failure policy.

## Minor
13. The "test plugin" makes plain Rust calls (no DLL, no exports, no wndproc);
    label it a surface test and keep separate VM-only DLL acceptance.

**Hard rules for every lane** clean room; no downloads; no `cargo add`; no git
write commands; **run no test** (the orchestrator runs the suite in the VM) —
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings` and `cargo build` are fine; no window may be
opened; every spawned child uses the no-window helper; English.
Each lane proves its files exist (paths + line counts read back from disk) and
reports what it changed, the tests, fmt/clippy, and the no-test/no-window/no-git
confirmations.
