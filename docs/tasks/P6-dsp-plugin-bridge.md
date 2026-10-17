# P6 — DSP plugin bridge for rustamp-plughost

## 1. Status and scope

- **Status**: Implemented, compile-checked only. No runtime pass claimed.
- **Boundary**: write only `crates/rustamp-plughost/**` and this card in the
  assigned worktree. Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research`
  or `D:\code\rustamp-refbin` was read or written. `crates/rustamp-plugin-abi/**`
  needed no change: `DspHeader`/`DspModule`, `DSP_VERSION` and
  `EXPORT_DSP_GET_HEADER` already existed.
- **Sources**: S09 (audited static, runtime pending), the P3/P4/P5 cards, and
  the existing clean-room loader/preflight/tests. No Winamp source, SDK
  material, disassembly or reference binary inspected; no third-party code
  copied, no dependency added, no license changed.

## 2. Spec facts (S09 section 6)

A classic DSP plugin (`DSP_*.DLL`) exports `winampDSPGetHeader2`
(undecorated, cdecl; S09 §§1–2). The 2.x header is 12 bytes
(version / description / `get_module`); the module is 32 bytes
(description / `parent_window` / `dll_instance` / `config` / `init` /
`process` / `quit` / `user_data`). `process` runs in place and returns the
frame count now in the buffer (S09 §6.2). Expected version is `0x20`;
`0x21`/`0x22` take a main-window argument and are 5.x-only.

### Settled by S09

1. Entry point name: `winampDSPGetHeader2` (not `winampDSPGetHeader2`-with-`2`
   confusion: the `2` is part of the name, §2).
2. Header 12 bytes in 2.x, 16 in 5.x; module 32 bytes.
3. `process` signature: `(module, samples, frames, bits, channels, rate)`,
   cdecl, returns output frame count.
4. In-place processing; output count between half and double the input.
5. `init` returns 0 on success; host fills `parent_window`/`dll_instance`
   before `init`; one module active at a time; `quit` at deselect/exit.

### Open in S09, decided here (own choices, not spec)

1. **U-7** (version acceptance): strict `== 0x20`. `0x21`/`0x22` are refused
   with `UnsupportedVersion`. Mirrors the general-plugin strict policy.
2. **U-9** (getter arguments): the getter is called with no arguments. A 5.x
   main-window argument would be a harmless extra under cdecl, so the no-arg
   call is correct either way.
3. **U-12** (DSP vs equalizer order): not decided by this bridge. No EQ stage
   lives in the host callbacks; `dsp_is_active` answers 1 only for a loaded
   DSP with a `process` callback, else 0.

## 3. Audio path: how samples reach the plugin and return

The input plugin owns its decode thread and its buffer (S09 §3.6). The
existing host callbacks handed to every input plugin are now DSP-aware:

```
input decode thread (owns the buffer, sized up to 2x when active):
  dsp_is_active()  -> 1 while a DSP module with `process` is installed, else 0
  dsp_process(samples, frames, bits, channels, sr)
      -> forwards to the active module's `process` on this same thread,
         in place, and returns the validated frame count
      -> downstream the input plugin feeds the (possibly resized) buffer to
         the vis/analyzer feed calls and the output `write` loop, as before
```

Position: the bridge sits *inside* the input plugin's decode loop, before
the vis/analyzer feeds. The active module is a process-wide slot
(`ACTIVE_DSP_MODULE`, UI thread installs at load, decode thread reads);
`process` receives all five stream arguments unchanged.

## 4. Failure modes: loud refusal, no player takedown

- **Refusal (`init` returns non-zero)**: `PluginLoadError::InitFailed { kind:
  Dsp, code }` before activation. The retained DLL mapping stays (P5/input
  policy) but no DSP becomes active and no caller can reach the module.
  Null header/module, null `get_module` (`MissingCallback`) and wrong
  versions fail the same way, loudly.
- **Crash in plugin code**: takes the 32-bit host process down with it —
  there is no in-process isolation (M3 crash isolation is a future
  milestone). The 64-bit player survives as a separate process; the bridge
  adds no new cross-process risk. Documented limitation, not handled.
- **Wedge (callback never returns)**: the decode thread blocks inside the
  synchronous `process` call; there is no watchdog or timeout in this
  bridge. Documented limitation, not handled.
- **Length change**: the plugin may return half..double; the bridge trusts
  that range exactly and falls back to the *input* count otherwise
  (`dsp_output_frames`), leaving the tail untouched. The cdecl contract
  offers the decode thread no error channel, so this fallback is silent at
  the audio path by necessity; misbehavior is observable only as "length
  unchanged" — garbage never propagates downstream. Missing `process`,
  null module/samples and non-positive input are a passthrough likewise.

Two deliberate deviations, both documented in code: `quit()` clears the
active slot but the mapping is retained (S09 §6.3 frees the DSP DLL after
quit; kept under the conservative input/general policy until the VM proves
free-after-quit safe), and a DSP without `process` loads but reads inactive.

## 5. Tests (headless; written, compile-checked, NOT executed here)

Synthetic in-memory test doubles inside `loader.rs` — no DLL is loaded,
no OS loader, window, audio or thread spawn:

| Test | What it proves |
|---|---|
| `dsp_gain_process_is_measurable_over_the_bridge` | ×2 gain double over the bridge is sample-exact |
| `dsp_without_process_or_addresses_is_a_passthrough` | null `process`/module/samples returns input count, buffer untouched |
| `dsp_output_frame_count_edges_are_honored_and_garbage_is_ignored` | half/double edges accepted; 49/201/0/negative/`i32::MAX` fall back |
| `dsp_lying_process_return_does_not_propagate` | 10x return collapses to the input count |
| `dsp_header_init_refusal_version_and_null_paths` | init=1 → `InitFailed`, slot stays empty; 0x21 refused; null header/getter/module refused; success fills host fields, installs the slot |
| `dsp_host_callbacks_follow_the_active_module` (windows) | `is_active` 0→1→0 with install/quit; stale clear is a no-op; silent module reads inactive |
| preflight structural fixture | `winampDSPGetHeader2` recognized as `PluginKind::Dsp` |

### What stays unproven without a DLL

Everything that needs executable plugin code in the i686 VM: export
resolution of a real `winampDSPGetHeader2`, the getter returning a live
header, version/description reads from plugin memory, real `init`/`process`/
`quit` execution and ordering, in-place buffer growth through a real input
plugin's decode loop, config dialogs, and the P3-style acceptance bundle.
**No such DSP DLL is available today, so the acceptance path stays unproven.**

## 6. Files and line counts

`splitlines` counts (code + blanks):

| File | Lines | Change |
|---|---|---|
| `crates/rustamp-plughost/src/loader.rs` | 1400 | DSP bridge: active slot, `init_dsp_module`, `call_dsp_process`, `dsp_output_frames`, `load_dsp_module`, two error variants, `Dsp` inner/quit, 6 tests |
| `crates/rustamp-plughost/src/pe_preflight.rs` | 580 | `winampDSPGetHeader2` recognized; message + selection comment + fixture test |
| `docs/tasks/P6-dsp-plugin-bridge.md` | this card | new (replaces the prior planning draft) |

No file under `crates/rustamp-audio`, `crates/rustamp-dsp`,
`crates/rustamp-core`, `crates/rustamp-mediahost` or any other crate was
touched. No `Cargo.toml`, `Cargo.lock` or `THIRD_PARTY.md` change (no new
dependency of any kind). No protocol or core contract change.

## 7. Build results

All run in the owned worktree (offline/locked where the tool supports it):

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 (final pass; one `cargo fmt --all` applied first) |
| `cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings` | exit 0, includes the new tests (two MSRV-gated findings fixed on the way: `err().expect()` → `expect_err()`, `const` thread-local init; `div_ceil` on `i32` is unstable in this toolchain, replaced with `/`+`%`) |
| `cargo build --workspace --offline --locked` | exit 0 |

These are default-target compile checks, not an i686 runtime or VM test pass.

## 8. Verification checklist

- [x] No test was run (compile checks only; new tests await the VM owner)
- [x] No library was loaded (synthetic bytes and in-memory doubles only)
- [x] No window was opened
- [x] No audio played
- [x] No download was performed, no dependency added
- [x] No commit was made, no git writes (`git status --short` below is the receipt)
- [x] All paths outside the worktree untouched; only owned paths changed
