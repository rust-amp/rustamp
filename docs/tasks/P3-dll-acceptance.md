# P3 — Real i686 plugin DLL acceptance (design; execution blocked)

**Status:** surface labels corrected; passive PE machine preflight implemented;
no DLL loader, probe DLL or executable acceptance runner implemented. No runtime
acceptance claimed. This card is not permission to run anything.

**Boundary:** write only your own files in
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave41-plughost-dll`.
Write nothing under `D:\code\Rustamp`; do not even read that canonical tree,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin`.
Owned paths: `crates/rustamp-plughost/**`, `crates/rustamp-plugin-abi/**`, this
card only. No other crate, protocol or core contract changes.

**Sources:** worktree copies of S09 (audited static, runtime pending), S10
(revised measured draft, independent ratification pending), RFC 0006, M3h
finding 13 and M3hC. No Winamp source, SDK material, disassembly or reference
binary inspected. No third-party code copied, dependency added or license
changed. Both owned crates remain MIT OR Apache-2.0; the host imports wire
state from `rustamp-proto`, never GPL `rustamp-core`. RFC 0006 §§2/7 explicitly
does not design classic DLL hosting or alter its messages; it supplies no
missing classic ABI fact.

## 1. Label correction and coverage boundary

`crates/rustamp-plughost/tests/test_plugin_ours.rs` is a **headless surface
test**, not a plugin. Its historical filename stays for existing references;
its module documentation and both function names now say surface/headless.
It directly calls `HostWindow::dispatch_ipc` / `dispatch_message`, uses made-up
handle numbers, and loads no DLL, resolves no export, invokes no ABI callback
and reaches no Win32 `wndproc`. Its setter assertion pins an implementation
fallback, not a measured sent-setter return (S10 E4/§13.11).

Misdescription locations before P3:

- The test's opening comment called it a "test plugin" and asserted replies
  a plugin could rely on; its first function was
  `our_test_plugin_exercises_the_surface_end_to_end`. Corrected in place.
- `src/host_window.rs`, `dispatch_ipc` documentation called it a "headless
  test plugin". Corrected to surface tests, explicitly excluding DLL/wndproc.
- `src/main.rs` claimed the helper "loads classic Winamp plugin DLLs" although
  `control.rs` ignores `LoadPlugin`. Corrected; the library overview agrees.
- Historical card `docs/tasks/M3b-host-window-message-loop.md` lines 7,
  30–34, 45–50 calls the fixture a plugin and says it proves end-to-end dispatch.
  **This P3 card supersedes that claim: M3b delivered surface coverage, not
  DLL acceptance.** The historical card is outside this lane's edit ownership;
  the integration owner must cross-link/correct it rather than cite it as proof.

The separate `rustamp-app/tests/plughost_e2e.rs` is app→host-window **surface
integration**, not DLL acceptance either. Thirteen external queries can prove
supervisor/state/window wiring, not a loaded plugin or an in-process caller.
M3hC §10 explicitly left that test's VM gate to its owning lane; do not assume
the gate is fixed merely because other robustness fixes exist. M3hC's
executable-machine check is about the host EXE, not plugin DLL validation.

## 2. Exact acceptance scope and reference gate

First milestone: load **one approved 32-bit general-purpose probe DLL** using
the production i686 host, exercise real calls and window procedures in both
directions, open/close its Config dialog, and quit cleanly. This proves that
specific DLL/kind, not all five kinds, audio, visualization rendering, embedding,
MilkDrop or arbitrary third-party compatibility.

The proposed fixture is our own clean-room `gen_` DLL, built only from S09,
with an undecorated export, static module and real Win32 windows. It must not
be statically linked into the runner or replace export calls with Rust calls.
A separately approved existing third-party DLL is a later compatibility case,
not a silent substitution for this instrumented fixture.

**A plugin binary is a reference artefact. Running even this proposed probe
requires explicit owner's approval for the exact binary and the reference
installation. Do not execute this design until that approval and installation
exist.** Downloads, installation and execution need separate explicit scope.
Reference 2.95 installation/execution and observation belong to an authorized
dirty-room lane; this implementation lane must not install/run/read it.
That lane must export audited spec facts before missing ABI behavior is coded.

Future execution additionally requires the disposable Hyper-V test VM, proved
before any spawn/load/window, plus an interactive guest session and display
proof for the dialog. `#[ignore]` alone is insufficient; no host override,
Sandbox, host window or audio. Pin checkpoint, fixture hash, reference build,
host build/target and installation manifest. Approval is not inferred from a
path, hash match, environment variable, successful preflight or a green build.

## 3. S09 fact ledger (no acceptance range invented)

| Fact used | Exact S09 source and evidence limit |
|---|---|
| Boundary is 32-bit x86; handles/pointers are 32 bits; C layout and 4-byte alignment; strings are NUL-terminated ANSI; module storage stays stable while loaded | §0 [295/SDK]; not the unused x64 Rust layouts |
| Entry points and struct callbacks are cdecl, exports undecorated C names; resolve only the expected kind's name | §1 [SDK/295] |
| Parent is the real `Winamp v1.x` main window; DLL instance is the plugin's own module handle, not the host EXE's | §1 [SDK/295] |
| `winampGetInModule2()` → static input module, expected 0x100 | §2/§3.2; exact 2.95 acceptance remains U-7, input recommendation [UNV] |
| `winampGetOutModule()` → static output module, expected 0x10 | §2/§4.1; wider acceptance U-7 |
| `winampVisGetHeader()` → static vis header, expected 0x101 | §2/§5.1; 0x102 and HWND argument are 5.x-only; actual 2.95 getter arguments U-9 |
| `winampDSPGetHeader2()` → static DSP header, expected 0x20 | §2/§6.1; 0x21/0x22 are 5.x-only; getter arguments U-9; acceptance U-7 |
| `winampGetGeneralPurposePlugin()` → static general module, expected 0x10 | §2/§7.1 [295]; 0x11 is 5.x-only; acceptance U-7 |
| General module is 28 bytes: version 0x00, description 0x04, init 0x08, config 0x0c, quit 0x10, parent 0x14, DLL instance 0x18 | §7.1 [295 layout]; init returns int, config/quit return void, all three take no arguments |
| Get module → check version → fill parent/instance → init; config on UI thread; quit in reverse load order, general DLL not freed at shutdown | §7.2 [5X, pattern 295]; **not runtime-proven 2.95 ordering/threading/unload policy** |
| General plugins may subclass main wndproc, create windows/threads; real replaceable procedures are required; Config/About use main/passed parent | §7.2, §8.6–8 [SDK/5X]; general Config parent/modal context is U-15 |
| Callback fields may be null unless stated otherwise | §0; representation as `Option` is not permission to call null. Input tolerance remains U-16; it does not establish mandatory general callbacks |
| IPC is main-window WM_USER/WM_WA_IPC 0x0400, id in LPARAM, argument in WPARAM | §8.2; synchronous return semantics from S10 §1.1–1.2 |
| Version id 0 → 0x2905 (10501), threshold 0x2900 | §8.3; external measurement aligned, in-DLL reference cross-check still open |
| End-of-track is a separate posted 0x0402 with both parameters zero | §3.6 [295 consistency, receiving behavior U-1]; **excluded** from this general/no-audio acceptance |

S09 does **not** require every general plugin to create a window, subclass or
provide Config. Those are explicit capabilities required of *our fixture* for
this test, not new ABI obligations. Require its init/config/quit slots to be
present; report a missing slot as fixture/precondition failure before that
callback, never transmute/call a null value or declare another plugin invalid
under an invented universal rule. Null descriptions need not be dereferenced.

## 4. Proposed VM call/observation sequence (not implemented)

Steps marked **policy/open** are test design choices or missing facts, not
observations of Winamp. Required ABI open questions below must be resolved or
the phase stays blocked; do not turn a proposed policy into a spec fact.

| Step | Host action and plugin reaction | Required observation / boundary |
|---|---|---|
| 0. Gate | Verify approval, reference installation, VM/session proof and hashes; build/stage real i686 host and probe; reject x64 substitution | Run manifest ties fresh run id to exact staged bytes. **Policy/open:** staging, DLL dependency search/isolation and authorization mechanism |
| 1. Start | Spawn through production supervisor with no-window creation flags; complete compatible Hello before state publication | Record real PID/target, Hello acceptance, UI heartbeat; use M3hC barrier/queue/budget/watchdog behavior, not a mock transport |
| 2. Parent | Create/own the host's actual hidden `Winamp v1.x` HWND before plugin init | Query class and owning PID/TID in guest. No placeholder handle; no surface borrow spans plugin/OS call |
| 3. Load | Inspect exact approved bytes' machine header, then a future vetted OS loader maps the DLL, resolves exactly `winampGetGeneralPurposePlugin` and calls it with no args, cdecl | OS module inventory: same DLL hash/path/base in that i686 PID. Trace resolved export address in mapped fixture executable code, call/return and non-null static module. **Policy/open:** secure loader/search, pointer validation, error handling; no loader exists now |
| 4. Validate | Check fixture version 0x10 and present init/config/quit, write host HWND and plugin HINSTANCE at S09 §7.1 offsets before init | Plugin independently logs received fields, checks parent ownership/class and own instance identity. Exact 0x10-only fixture policy is **not** proof of S09 U-7 acceptance range |
| 5. Init | Call `init()` on UI thread; fixture returns 0, subclasses host wndproc and creates its own identifiable window | Pair host enter/return with DLL-origin trace, actual plugin-window HWND/PID/TID/class and capture. **Open:** S09 U-8 nonzero semantics; general lifecycle/thread equivalence to 2.95 remains unconfirmed |
| 6. Query | From DLL code, send `WM_WA_IPC` synchronously to parent, wParam 0, lParam 0; subclass logs entry and forwards to saved original proc, returning its result unchanged | Host original wndproc trace + plugin before/after trace must agree on raw u32 10501 (0x2905), comparison >=0x2900 true; no direct `dispatch_ipc` call |
| 7. Fallback | DLL sends WM_WA_IPC with wParam 0, lParam 9999 and forwards through saved proc | Return 1 from S10 §1.5/§3 (refutes S09 §8.4/U-6 guess 0); do not invent an additional classic id |
| 8. Plugin wndproc | Runner requests the host to send a fresh challenge to the plugin-owned window; that DLL's wndproc records and acknowledges it | Correlated host-send / DLL-wndproc-receive / reply records and same PID/window. **Policy/open:** private fixture-only OS-registered message + challenge protocol, not a Winamp IPC number; no numeric id assigned here |
| 9. Config | User-equivalent Configure request reaches UI thread; host calls `config()` with no args; fixture creates a config dialog owned by supplied parent, closes on explicit guest action and returns | Host/DLL callback pair, distinct dialog HWND, owner relation, PID/TID, before/open/closed captures and responsive message pumping. **Open:** U-15 modal/parent behavior and routing a Configure request; proposed fixture dialog behavior is policy, not 2.95 fact |
| 10. Quit | Host calls `quit()` before parent destruction; fixture destroys dialog/window and restores subclass chain | DLL quit enter/exit, windows gone, original proc restored, no later fixture callbacks, bounded host exit. Retain general DLL until process exit per §7.2 [5X]; **open:** actual 2.95 unload policy; no premature FreeLibrary |

Do not infer process health from reader-thread Pong while a DLL/UI thread is
hung. Deadline/crash/parent-death outcomes are failures, with preserved traces,
not successful cleanup. No success record if any required stage is missing.

For state-dependent extension, publish fresh non-default volume/pan snapshots
through the real parent and query ids 122/123 with signed -666 in the DLL,
recording raw 32-bit replies (S10 §§1.5/5.5/5.6). This is **additional surface
integration**, not required for the minimal DLL proof. Do not assert sent
setter returns, raw-Hz id 126, pointer contents, EQ/PE/menu handles or id-800
callability: M3h Group B/S10 leave these open. Never call the measured foreign
address 0x434200 or treat unsupported value 1 as a function pointer.

## 5. Missing work and spec requests

1. **Owner/reference gate:** name approved probe and reference binaries/hashes,
   permission scope, authorized dirty-room operator and installed VM reference.
   No approved executable probe or reference run is supplied by this card.
2. **Fixture + real loader:** clean-room cdylib/export build, vetted OS loading,
   dependency search containment, module lifetime ownership, ABI call wrapper,
   callback/pointer validation and load/error diagnostics. Preflight below is
   deliberately not wired into `LoadPlugin`; that message is still ignored.
3. **Control routing:** dispatch approved load/config/quit requests onto UI
   thread, report actual errors/loaded status, and collect evidence without
   corrupting framed stdout. Wire changes, if needed, require a separate RFC
   and owner lane; `PluginLoaded` alone is not acceptance evidence.
4. **S09 confirmations:** U-7 version acceptance, U-8 general init failure,
   U-15 Config context; generalized null-callback and general unload/thread
   tolerance. Current [5X] ordering is a candidate, not a 2.95 finding. Use
   valid 0x10/zero-init fixture only; defer negative init semantics until spec.
5. **Private test protocol/evidence:** choose challenge transport, fixture
   window identity and dialog automation, trace schema/flush rules/deadlines,
   module-address attribution and independent guest observation. S09 settles
   none of these test mechanisms; no private numeric ABI is introduced here.
6. **Other kinds:** vis acceptance requires `get_module(index)`/null handling,
   host-filled fields, init/render/config/quit, row counts and window ownership
   from S09 §5; U-2/U-9 threading/getter args, U-3 data and U-5 discovery remain
   open. DSP/input/output need their own lifecycle tests. MilkDrop's 5.x
   dependencies are not a 2.95 baseline. S11 embedding is out of this card.
7. **Reference cross-check:** authorized dirty room loads the approved probe
   into installed 2.95 and exports audited id-0 results; clean VM run loads it
   into Rustamp. Rustamp answering 0x2905 alone cannot validate the reference
   constant. S10 §13.1 stays open until both directions are evidenced.

## 6. Safe scaffolding available now

`src/pe_preflight.rs::check_pe_i386_machine(&[u8])` checks MZ, declared PE offset,
PE signature, complete COFF header and machine 0x014c. It refuses other machines
with their actual hex value, malformed/truncated headers, overlapping/out-of-
bounds offsets and arithmetic overflow. Bounded slice access; no unsafe code,
allocation in the parser, file access, DLL load, symbols, callbacks or execution.
PE field facts are listed in M3hC §11; no GPL application's code was copied or
imported. No new dependency was permitted for this narrow header check.

Seven headless test functions use synthetic byte arrays only: header match,
x64/ARM/unknown rejection, every truncated prefix, both signatures, overlapping/
huge offsets, declared-offset semantics and architecture diagnostics. They are
**written/compile-checked, not executed in this lane**; the VM owner must run
them. An accepted synthetic header intentionally has no optional header or
sections: this proves the check is *not* a DLL validator. It does not check DLL
characteristics, imports, exports, executable sections, PE32 optional header,
signature/trust or CPU instructions; it is neither a loader nor an execution
authorization token. Future loading must validate/use the same approved bytes,
not preflight one pathname and later load a replaced file (TOCTOU).

## 7. Evidence that passes, and false passes

Acceptance artefact: a retained **VM run bundle** containing owner approval
reference, checkpoint/session proof, run id + fresh challenge, build/target and
binary hashes, module inventory proving the DLL mapped into the real i686 host,
attributed export/callback traces, plugin and host wndproc challenge/reply traces,
version/unknown IPC replies, window/dialog PID/owner metadata and captures,
quit/subclass restoration and runner exit/results. Records must share run id,
PID and exact fixture identity; missing/unflushed/truncated phases fail. Bundle
collection/storage is proposed, not implemented or produced in this lane.

A valid runtime pass requires **every** step 0–10, independently observed DLL
code execution and real window-message delivery, not just a screenshot or log
line printed by the runner. Pin a nonzero executed-test/phase count; skipped,
ignored, gated-off, wrong-binary, timed-out or zero-test runs are not passes.
Negative controls (future approved VM run) must fail for wrong-machine bytes,
missing/decorated export, null module, unsupported fixture version, missing
required fixture slot, absent acknowledgement/dialog and DLL crash/hang. For
pre-entry failures, require no init record; do not guess 2.95 init-error rules.

False passes include this headless surface test; external `plughost_e2e` alone;
`PluginLoaded`/Hello/Pong alone; the PE header check; mocked/statically linked
callbacks; merely finding an export name; a host-created substitute plugin
window; placeholder HWNDs; a dialog in another process; stale traces/captures;
or treating one `gen_` success as support for every classic plugin kind.

## 8. Lane verification

Allowed compile-only commands: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo build --workspace` (offline/locked where available). Run no tests here,
including headless tests. No downloads, new dependencies, reference inspection,
plugin execution, windows, audio or git writes. Report command results and
worktree-local `git status --short`; runtime acceptance remains blocked.

2026-10-17 lane receipts: fmt check **exit 0**; workspace all-target/all-feature
Clippy with `--offline --locked -- -D warnings` **exit 0**; workspace build
with `--offline --locked` **exit 0**; `git diff --check` **exit 0**. These are
default-target compile checks, not an i686 runtime proof. Seven new test
functions were compile-checked only. No test ran, plugin loaded, window opened,
audio played, download performed or commit made. All authored source/card
changes are in the owned paths above; build outputs remain worktree-local.
