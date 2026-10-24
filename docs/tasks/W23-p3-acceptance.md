# W23 — P3 plugin DLL acceptance, taken forward under the owner's release

**Status:** inventory written first, then the open acceptance work in the owned
paths implemented. Compile gates run here; **no test ran in this lane** and no
runtime acceptance is claimed from it. The Group D / VM-only verdicts belong to
the orchestrator's interactive VM run.

**Release:** the owner released everything on 2026-10-18
(`docs/notes/lane-jobs.md`, "The owner released everything"; `docs/tasks/LANE-QUEUE.md`
"approved by the owner, 2026-10-18"). `docs/tasks/P3-dll-acceptance.md:5`
("This card is not permission to run anything") and its §2 gate paragraph were
true when written and are **superseded**. This card does not rewrite P3; P3
stays the design of record and is read here as a work list, not a prohibition.

**Owned paths:** `crates/rustamp-plughost/**`, `crates/rustamp-plugin-abi/**`,
this card. Nothing else was touched.

---

## 1. What P3 left open, and what the code already does

Read before changing anything. File and line as of this lane's start commit
(`b26df3c`).

### Already done — do not rebuild

| P3 item | Where it already is |
|---|---|
| §5.2 real loader | `crates/rustamp-plughost/src/loader.rs:720` `load_plugin`, `:761` `load_plugin_with_store`, `:788` `map_plugin_image` (sealed private snapshot, `AddDllDirectory` + `LOAD_LIBRARY_SEARCH_*`, never the source pathname), `:829` `load_plugin_x86` (export resolution per kind), `:901` input, `:970` DSP, `:997` general |
| §6 passive preflight | `src/pe_preflight.rs:87` `check_pe_i386_machine`, `:121` `check_pe_i386_dll` (PE32/DLL structure, sorted export table, non-forwarded executable getter), export→kind map at `:244`–`:246` |
| §2/§7 byte approval, no path trust | `src/approval.rs` (`ApprovalStore`, SHA-256 + machine + size tuple, malformed store refuses loudly, never falls back to an empty allowlist), `src/loader/image.rs` (`PreparedImage`, seal, ancestor pinning, TOCTOU) |
| Group D, input kind, real DLL | `tests/dll_acceptance.rs:110` `loads_real_32bit_in_openmpt_dll_and_proves_host_interaction` — VM-only, `#[ignore]`, loads the real `in_openmpt.dll`, proves mapped-snapshot identity, `winampGetInModule2`, version, description, IPC 0 → `0x2905` |
| Group D, DSP kind, real DLL | `tests/dsp_acceptance.rs:108` `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge` — VM-only, `#[ignore]`, pinned SHA-256 fixture, tampered-byte negative control, `winampDSPGetHeader2`, host-filled fields, `process` measured, `quit` deactivates |
| DSP bridge and its host callbacks | `src/loader.rs:281` `dsp_output_frames`, `:313` `call_dsp_process`, `:346` `init_dsp_module`, `:604` `dsp_info`, `:627` `dsp_process_i16` |
| §1 label correction | `tests/test_plugin_ours.rs` is a headless **surface** test (no DLL, no wndproc); `src/host_window.rs` says so |
| §5.1 owner/reference gate | released, see above |

### Still open at this lane's start

| P3 item | Evidence it is open |
|---|---|
| §5.3 **control routing** | `src/control.rs:135` and `:160` drop every other variant; `src/control.rs:56`–`66` documents `LoadPlugin` as deliberately ignored; `src/main.rs:5` still claims "no DLL loader or export invocation exists, and `LoadPlugin` is ignored"; `src/lib.rs:7` still claims "No DLL-loading path exists yet". Both doc claims were **false** once the loader landed. Nothing turned a `LoadPlugin` request into a load, a `PluginLoaded` or a `PluginError` |
| §4 step 9 **`config()`** | no code path invoked a plugin's `config` callback for any kind. `grep config` in `loader.rs` hits only `DspModuleInfo::has_config` (`:612`, `:674`), documented as "never invoked by the host here" |
| §4 step 4 **general slot/field reporting** | `load_general_module` (`src/loader.rs:997`) checks version (`:1013`), writes `parent_window`/`dll_instance` (`:1029`–`:1032`) and calls `init` (`:1035`), but exposed nothing: no general counterpart to `dsp_info`, so a runner could not require the fixture's `init`/`config`/`quit` slots **before** calling them, and a null `init` was silently skipped while the plugin was still reported `initialized` |
| §4 step 10 **reverse-order quit** | `loader.rs:30` documents "quit in reverse order", but only a single `LoadedPlugin::quit` (`:557`) and its `Drop` (`:689`) existed. No ordered multi-plugin owner, so no reverse order could be observed |
| §5.6 **other kinds** | `pe_preflight.rs:244`–`246` recognises input/DSP/general only; `loader.rs:855`/`:871` `unreachable!("preflight selects input/dsp/general only")`. Visualization and output are unimplemented |
| §5.5 private test protocol | no challenge transport, fixture window identity or trace schema exists |
| §5.4 S09 confirmations | spec lane, not code. `loader.rs:1036` still records U-8 as unverified |
| §5.7 reference cross-check | dirty-room lane |
| `drain` doc vs behaviour | `src/message_loop.rs:123` `drain` says "once per queued message until it drains" and pumped exactly once |

---

## 2. What this lane implemented

All in the owned paths, all from S09 §7 and P3 §4 as read above.

1. **General-purpose acceptance surface** (`src/loader.rs`)
   - `init_general_module` factored out of `load_general_module` so the module
     pointer, version refusal, host-field write and `init` call are reachable
     from a test with a synthetic module, exactly as `init_dsp_module` already
     was. Returns what actually happened: whether `init` existed, and the raw
     `int` it returned, recorded and **not** interpreted (S09 U-8 stays open).
   - `GeneralModuleInfo` + `LoadedPlugin::general_info()`: the `init`/`config`/
     `quit` slots as the plugin supplied them, `parent_window`/`dll_instance`
     read back out of the plugin's own struct, `init_called` and `init_result`.
     This is what lets an acceptance runner satisfy P3 §3 — *require* the
     fixture's slots and report a missing one as a precondition failure **before**
     that callback — without the loader inventing a universal rule that rejects
     a general plugin S09 does not require to have a `config`.
   - `LoadedPlugin::config()` + `PluginConfigError`: calls the general plugin's
     `config` callback with no arguments on the calling (UI) thread. A null slot
     returns `MissingCallback` and **never** transmutes or calls null; a plugin
     already quit returns `NotInitialized`; any other kind returns `WrongKind`
     (a DSP `config` opens a dialog and stays deliberately uninvoked, as
     `tests/dsp_acceptance.rs:20`–`23` says).
2. **Ordered registry and control routing** (`src/registry.rs`, new)
   - `PluginRegistry` owns the approval decision and the loaded plugins **in
     load order**, assigns the `plugin_id` the protocol already carries, and
     quits in **reverse** load order on `quit_all`/`Drop` (S09 §7.2, P3 §4
     step 10). The DLLs stay mapped (S09 §1 line 46).
   - `handle_load_request` turns a `Message::LoadPlugin` into exactly
     `Message::PluginLoaded` or `Message::PluginError` — both already in the
     protocol, so **no wire change and no RFC is needed** for this step.
     Order: approval source available → expected kind vs. the kind passive
     preflight selects (refused before any mapping, so no DllMain runs for a
     wrong-kind request) → `load_plugin_with_store` → the authoritative
     post-load kind check (`accept_loaded_kind`) → id assignment.
   - A **missing or malformed approval store is never an empty allowlist**: the
     registry carries the reason and answers every request with a `PluginError`
     naming it. The default with no store is an empty store, which approves
     nothing.
3. **`ControlInbound::Load`** (`src/control.rs`): `LoadPlugin` is routed to the
   UI thread instead of dropped. `handle_message` (the reader-thread variant
   that owns the transport) still does not act on it: a load must happen on the
   thread that owns the window, and that variant has no window.
4. **UI-thread dispatch** (`src/main.rs`): `--approvals <path>` selects the
   approval store (absent → approves nothing); the UI loop handles a `Load`
   event with the registry and sends the reply through the existing outbox
   channel, so the framed stdout is never corrupted by a print.
5. **Bounded pumping** (`src/message_loop.rs`): a new `pump_step` →
   `PumpStep::{Dispatched, Empty, Quit}` distinguishes an empty queue from a
   dispatched message, `pump_bounded` drains up to a budget and stops at the
   first empty queue, and `drain` uses it — so the function now does what its
   name and docs already claimed, which is the responsive pumping P3 §4 step 9
   needs around a modal config dialog. `pump_once` keeps its old signature and
   meaning. **No sleep was added anywhere**, and nothing spins on an empty
   queue.
6. **Stale claims corrected**: `src/lib.rs` and `src/main.rs` no longer say the
   loader does not exist or that `LoadPlugin` is ignored.

### Tests, and how each one can fail

Host-side tests (ordinary suite, any Windows target):

| Test | Fails when |
|---|---|
| `loader.rs` `general_version_refusal_precedes_any_callback` | a wrong `version` loads, or `init` runs before the version check (the synthetic `init` records that it ran) |
| `loader.rs` `general_host_fields_are_visible_to_init_and_recorded` | `parent_window`/`dll_instance` are not the host's values **as seen from inside `init`**, or `init_result` is not the value the plugin returned |
| `loader.rs` `general_null_module_and_null_init_are_reported_not_called` | a null module loads, or a null `init` is reported as called |
| `loader.rs` `general_init_refusal_is_recorded_and_not_turned_into_a_verdict` | a non-zero general `init` becomes a refusal (S09 U-8 is open), or the plugin's own value is lost |
| `loader.rs` `general_config_runs_once_and_a_null_slot_is_refused` | `config` is not invoked, is invoked twice, or a null `config` is called instead of refused |
| `loader.rs` `general_config_after_quit_and_on_other_kinds_is_refused` | `config` runs on a quit plugin or on a non-general kind |
| `loader.rs` `general_quit_runs_once_and_is_idempotent` | `quit` is skipped or runs twice |
| `registry.rs` `an_unavailable_approval_store_refuses_every_load_with_its_reason` | a malformed store degrades to approving nothing silently, or the reason is lost |
| `registry.rs` `an_empty_store_approves_nothing_even_for_a_structurally_valid_dll` | an empty allowlist loads a preflight-clean i386 DLL |
| `registry.rs` `a_wrong_expected_kind_is_refused_before_any_mapping` | the kind mismatch is noticed only after DllMain ran, or not at all |
| `registry.rs` `a_missing_file_reports_the_real_io_error` | the error is swallowed or replaced by a generic message |
| `registry.rs` `accept_loaded_kind_is_authoritative_after_the_load` | the post-load kind check accepts a mismatch |
| `registry.rs` `plugins_quit_in_reverse_load_order` | quitting is forward, partial, or repeated |
| `registry.rs` `config_reaches_exactly_the_named_plugin_and_an_unknown_id_touches_nothing` | `config` reaches the wrong plugin, an unknown id reaches any plugin, or a null slot is called |
| `registry.rs` `a_fresh_registry_holds_nothing_and_hands_out_ids_from_one` | a fresh registry claims plugins, or hands out id 0 |
| `control.rs` `a_load_request_is_routed_to_the_ui_thread_and_answered_nowhere_else` | `LoadPlugin` is dropped again, or the reader thread answers it on the transport |

VM-only test (`tests/control_load_acceptance.rs`, `#[ignore]`):

| Test | Fails when |
|---|---|
| `the_real_host_process_loads_an_approved_dll_through_the_control_channel` | the production `rustamp-plughost32.exe` cannot be found, the framed `Hello` never arrives, a `LoadPlugin` for the approved real `in_openmpt.dll` does not come back as `PluginLoaded` with kind `Input` and a non-empty name, the unapproved negative control is **not** refused with `PluginError`, the stdout frames do not decode, or the child does not exit 0 on `Shutdown` |

It refuses loudly (never skips) when it is not in the test VM, not on i686, when
the host binary is not staged, or when the plugin fixture is missing. Its
deadlines are `recv_timeout` on a channel fed by a reader thread — a deadline on
a condition, not a sleep used to order anything.

### What only the virtual machine can produce

Nothing in this lane ran. Specifically:

- every `#[ignore]`d test above, old and new, is the orchestrator's interactive
  VM run to execute: they create real windows, map real 32-bit DLLs and spawn
  the real i686 host process;
- the new `tests/control_load_acceptance.rs` additionally needs the i686
  `rustamp-plughost32.exe` staged beside the test binaries (the runner already
  does this, `docs/notes/vm-run-tests.ps1` lines 200–241) and
  `RUSTAMP_TESTPLUGINS` pointing at `in_openmpt.dll`;
- the host-side tests listed above need no VM, but this lane did not run them
  either — `cargo test` belongs to the orchestrator.

---

## 3. Still open after this lane

1. **An approved `gen_` fixture DLL, and the crate that builds it** (P3 §2,
   §5.2). P3's first milestone is one general-purpose DLL loaded end to end with
   its Config dialog. The loader, `general_info`, `config()` and the reverse-order
   quit are now in place, but **there is no `gen_` binary to load**: the two
   approved fixtures are `in_openmpt.dll` (input) and `dsp_optilab_core.dll`
   (DSP). A clean-room `gen_` cdylib needs a **new workspace member**, and
   `Cargo.toml`'s `members` list (lines 3–28) is outside this lane's owned paths.
   **Change needed outside the owned paths:** add a fixture crate (e.g.
   `crates/rustamp-gen-fixture`, `crate-type = ["cdylib"]`, undecorated
   `winampGetGeneralPurposePlugin`, real Win32 window, subclassed host wndproc,
   own Config dialog) to the workspace members, built for
   `i686-pc-windows-msvc`. It must **not** be a target of `rustamp-plughost`:
   a fixture statically linked into the runner is one of P3 §7's false passes.
   Until it exists, no general acceptance test was written — a VM test that can
   only refuse for a missing fixture would be a red in the orchestrator's run,
   not a test.
2. **Steps 6–8: DLL-origin messages** (P3 §4). Both existing Group D tests send
   their IPC query from the *runner*, not from plugin code (`tests/dll_acceptance.rs:42`
   `send_ipc_query`). Proving "from DLL code, send `WM_WA_IPC` to the parent and
   forward through the saved original proc" needs the fixture in item 1, plus
   the private challenge protocol of §5.5. Nothing here claims that step.
3. **The app side of the control channel.** `rustamp-app` never sends
   `LoadPlugin` and still does not read the host's stdout (`src/control.rs:46`–`54`
   records this). The host half is now implemented and testable on its own; the
   app half, including passing `--approvals`, is **outside the owned paths**.
   **Change needed outside the owned paths:** `rustamp-app/src/host.rs` /
   `supervise.rs` to send `LoadPlugin`, pass `--approvals`, and read
   `PluginLoaded`/`PluginError`.
4. **A `config`/`quit` request over the wire.** `handle_load_request` needed no
   protocol change because `LoadPlugin`/`PluginLoaded`/`PluginError` exist. There
   is **no** message for "configure plugin N" or "quit plugin N", so routing a
   user-equivalent Configure request from the app (P3 §4 step 9) **does** need an
   RFC and the contract lane. `PluginRegistry::config` is ready for it in-process.
5. **Visualization and output kinds** (P3 §5.6). Deliberately untouched: adding
   `winampVisGetHeader` to `pe_preflight` without a vis bridge would make
   `loader.rs:855`/`:871` reachable and panic. The vis lifecycle also still
   rests on S09 U-2/U-9 (threading, getter arguments) and U-3 (waveform data),
   and no vis fixture is approved. It needs its own card.
6. **The run bundle** (P3 §7). Trace schema, module inventory, window/dialog
   metadata and captures are still proposed, not produced. The new VM test
   prints its evidence to the test log; that is not the retained bundle.
7. **Ignored-test inventory.** `docs/tasks/W12-ignored-inventory.md` /
   `W19-ignored-readiness.md` list the `#[ignore]`d tests; the new
   `control_load_acceptance` test belongs in that list. `docs/tasks/**` other
   than this card is outside the owned paths. **Change needed outside the owned
   paths:** add it there.
8. **M3b cross-link** (P3 §1). `docs/tasks/M3b-host-window-message-loop.md` still
   calls the surface fixture a plugin; the integration owner must correct it.

---

## 4. Lane verification

Allowed compile-only commands, run in this worktree. No test ran, no VM was
used, no broker was invoked, no download happened, no dependency was added and
nothing was committed.

- `cargo fmt --all -- --check` → **exit 0**
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` → **exit 0**
- `cargo build --workspace --offline` → **exit 0**
- `git diff --check` → **exit 0**

Two extra compile-only checks, because the host's real target is i686 and the
default-target gates above would not catch an arch-dependent mistake:

- `cargo build -p rustamp-plughost --target i686-pc-windows-msvc --offline` → **exit 0**
- `cargo clippy -p rustamp-plughost --all-targets --all-features --target i686-pc-windows-msvc --offline -- -D warnings` → **exit 0**

`git status --short` in this worktree: modified `crates/rustamp-plughost/src/{control,lib,loader,main,message_loop}.rs`,
new `crates/rustamp-plughost/src/registry.rs`,
new `crates/rustamp-plughost/tests/control_load_acceptance.rs`,
new `docs/tasks/W23-p3-acceptance.md`. Nothing outside the owned paths.

Runtime acceptance is still the VM's to produce.
