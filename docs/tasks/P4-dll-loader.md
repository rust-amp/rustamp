# Task card P4 — Real 32-bit Winamp Plugin DLL Loader and Acceptance Test

## 1. Status and Scope

- **Status**: Implemented. The clean-room 32-bit plugin DLL loader is implemented in `crates/rustamp-plughost/src/loader.rs`, integrated with `HostWindow` message recording in `crates/rustamp-plughost/src/host_window.rs`, and exercised by the VM-only acceptance test `crates/rustamp-plughost/tests/dll_acceptance.rs`.
- **Approved target binary**: `in_openmpt.dll` (freely licensed libopenmpt 0.8.9 BSD-3-Clause build located at `D:\code\rustamp-testplugins\libopenmpt\Winamp\in_openmpt.dll`, with dependent `openmpt-mpg123.dll` beside it).
- **Execution constraint**: No plugin or reference binary has been executed on the development host. Execution is strictly gated to the disposable Hyper-V test VM via `require_test_vm()`.
- **Existing tests**: `crates/rustamp-plughost/tests/test_plugin_ours.rs` remains labelled as a headless surface test; it is not presented as DLL acceptance.

## 2. ABI Facts Implemented and S09 Line Numbers

All ABI structures, constants, offsets, and behaviors are derived strictly from `specs/S09-plugin-abi.md` and `specs/S10-ipc.md`:

| Topic | ABI Fact | S09 Reference |
|---|---|---|
| Architecture & Layout | 32-bit x86 Windows process; 4-byte alignment, zero padding; pointers and handles are 32 bits wide; structures live in plugin static memory. | S09 §0, lines 28–32 |
| Calling Convention | Entry-point exports and structure callbacks use `cdecl` (`extern "C"`, caller cleans stack). Exports are undecorated C names. | S09 §1, line 38 |
| Version Check | First member of every module/header is integer version. Host refuses unsupported versions. | S09 §1, line 41 |
| Parent Window | Host fills `main_window` with handle of `Winamp v1.x` main window before `init()`. | S09 §1, line 42; §3.1 offset 0x08 |
| Module Handle | Host fills `dll_instance` with the plugin's own `HINSTANCE`. | S09 §1, line 43; §3.1 offset 0x0C |
| Unload Policy | Input, output, and general purpose DLLs are not freed at shutdown (preventing crashes from subclassed window procedures). | S09 §1, line 46; §3.5 line 166 |
| Input Entry Point | `winampGetInModule2` taking no arguments, returning `*mut InModule`. Expected version `0x100`. | S09 §2, line 56 |
| General Entry Point | `winampGetGeneralPurposePlugin` taking no arguments, returning `*mut GeneralPlugin`. Expected version `0x10`. | S09 §2, line 60; §7.1 line 367 |
| InModule Layout | 38 members, 152 bytes (0x98): version 0x00, description 0x04, main_window 0x08, dll_instance 0x0C, file_extensions 0x10, seekable 0x14, flags 0x18, config 0x1C, about 0x20, init 0x24, quit 0x28, get_file_info 0x2C, info_box 0x30, is_our_file 0x34, play 0x38, pause 0x3C, unpause 0x40, is_paused 0x44, stop 0x48, get_length 0x4C, get_position 0x50, seek 0x54, set_volume 0x58, set_pan 0x5C, analyzer_init 0x60, analyzer_deinit 0x64, analyzer_feed_pcm 0x68, analyzer_get_mode 0x6C, analyzer_feed_rows 0x70, vis_feed_pcm 0x74, vis_get_mode 0x78, vis_feed_frame 0x7C, vis_set_format 0x80, dsp_is_active 0x84, dsp_process 0x88, eq_set 0x8C, set_info 0x90, out_module 0x94. | S09 §3.1, lines 74–117 |
| Input Version & Flags | Base version 0x100 (`IN_VERSION`). Wide-character filenames flag `0x0F00_0000` masked for 5.x builds (e.g. `in_openmpt 0.8.9` build `0x0F000100`). | S09 §3.2, lines 121–131; §11 line 437 |
| Host Field Filling | Host writes `main_window`, `dll_instance`, `analyzer_*`, `vis_*`, `dsp_*`, clears `set_info` before `init()`, and sets `set_info` after `init()` returns. Sets `out_module` to null before play. | S09 §3.5, lines 153–156 |
| Host Callback Semantics | `analyzer_get_mode` returns 4 (none/window-shade); `analyzer_feed_rows` returns 1 (not collecting); `vis_get_mode` zeroes row outputs and returns 0; `vis_feed_frame` returns 1 (not accepted); `dsp_is_active` returns 0; `dsp_process` returns frame count. | S09 §3.1, lines 102–115 |
| Input Lifecycle | Host startup: load DLL, call `winampGetInModule2`, check version, fill host fields, call `init()`. Host exit: call `quit()`, DLL not freed. | S09 §3.5, lines 151–167 |
| General Lifecycle | Host startup: load DLL, call `winampGetGeneralPurposePlugin`, check version 0x10, fill parent/instance, call `init()`. Exit: call `quit()`, DLL not freed. | S09 §7.1, §7.2 lines 375–384 |
| IPC Channel | Message `WM_USER` (0x0400, `WM_WA_IPC`) sent to main window (`Winamp v1.x`), IPC id in `LPARAM`, argument in `WPARAM`. | S09 §8.2, line 388 |
| Version Query | `IPC_GETVERSION` (0) sent to main window returns `0x2905` (10501) per RFC 0009 and measured S10 runs. | S09 §8.3, line 389; S10 §3.1 |
| Unsupported IPC | Unknown/unsupported IPC numbers answer `1` per measured S10 findings (refuting S09 U-6 guess of 0). | S10 §1.5, lines 171–178 |

## 3. Implementation Summary

1. **`crates/rustamp-plughost/src/loader.rs`**:
   - `load_plugin(&Path, &HostWindow) -> Result<LoadedPlugin, PluginLoadError>`:
     - Passive PE machine preflight on image bytes via `check_pe_i386_machine` (requires machine `0x014c`).
     - Rejection if not running in a 32-bit x86 target (`cfg!(target_arch = "x86")`).
     - Windows `LoadLibraryExW` with `LOAD_WITH_ALTERED_SEARCH_PATH` (0x00000008) so dependent DLLs in the plugin directory are loaded cleanly.
     - Undecorated export resolution (`winampGetInModule2` and `winampGetGeneralPurposePlugin`).
     - Invocation of cdecl entry point with no arguments.
     - Verification of non-null module pointer and module version.
     - Population of host fields and callbacks (`main_window`, `dll_instance`, `analyzer_*`, `vis_*`, `dsp_*`, `out_module`).
     - UI-thread execution of `init()`, and assignment of `set_info` after `init()` returns.
   - `LoadedPlugin`:
     - Holds module pointer, `HINSTANCE`, kind, description.
     - `quit()` calls the plugin's `quit()` callback.
     - `Drop` ensures `quit()` is invoked without freeing the DLL (per S09 §1 line 46).

2. **`crates/rustamp-plughost/src/host_window.rs`**:
   - `WindowState` augmented with `ipc_queries: RefCell<Vec<(u32, i64, u32)>>` and `window_messages: RefCell<Vec<(u32, usize, isize)>>`.
   - Window procedure logs all received `WM_USER` IPC queries `(ipc_id, wparam, reply)` and window messages.
   - `HostWindow` exposes `recorded_ipc_queries(&self)` and `recorded_messages(&self)`.

3. **`crates/rustamp-plugin-abi/src/lib.rs`**:
   - Added `PluginKind::from_path(path: &Path) -> Option<PluginKind>` helper.

4. **`crates/rustamp-plughost/tests/dll_acceptance.rs`**:
   - VM-only integration test `loads_real_32bit_in_openmpt_dll_and_proves_host_interaction`.
   - Gated with `require_test_vm()` (refuses loudly outside Hyper-V test VM).
   - Refuses loudly if missing `in_openmpt.dll` across environment variable `RUSTAMP_TESTPLUGINS`, guest staged paths, and reference path.
   - Refuses loudly if run on a 64-bit target.
   - Verifies:
     1. Entry point ran and returned valid `InModule` with description identifying `in_openmpt` / `libopenmpt`.
     2. Plugin messages arrived at the host window.
     3. Host answered at least one IPC query synchronously (including `IPC_GETVERSION` -> `0x2905`).
     4. Clean teardown via `loaded.quit()` and `window.destroy()`.

## 4. What the VM Run Must Show vs What It Cannot Show

### What the VM Run Must Show:
1. **Machine & Preflight Verification**: Passive PE preflight approves the 32-bit x86 COFF machine (`0x014c`) of `in_openmpt.dll`.
2. **OS Mapping**: Windows dynamic linker maps the 32-bit DLL into the i686 process address space, resolving `openmpt-mpg123.dll` via `LOAD_WITH_ALTERED_SEARCH_PATH`.
3. **Export Resolution**: Symbol `winampGetInModule2` is located in the export table.
4. **Cdecl Entry Point Execution**: Calling `winampGetInModule2()` succeeds without argument passing, returning a non-null static `InModule` pointer.
5. **Static Module Validation**: Version is `0x100` (or `0x0F000100` with wide-character flag); description string is non-empty and read from plugin memory.
6. **Host Field Acceptance**: Plugin accepts host HWND and host function pointers.
7. **Plugin Initialization**: Plugin executes `init()`.
8. **Host Window Message Delivery & IPC Query Answering**: During `init()` or message pumping, `in_openmpt` sends messages/queries to `HostWindow` (`Winamp v1.x`), and `HostWindow` answers synchronously (e.g. `IPC_GETVERSION` answers `0x2905`).
9. **Reverse Lifecycle Teardown**: Calling `quit()` runs cleanly and `HostWindow` destroys without memory corruption or dangling pointer access.

### What the VM Run Cannot Show:
1. **Audio Playback**: Does not show audio decoding loops, decoded PCM verification, or device output; `in_openmpt.dll` playback requires feeding paths and driving `play()`, `seek()`, `stop()`, which belongs to later media host cards.
2. **Interactive UI Window**: An input plugin does not open a standalone visual rendering window during startup (unlike visualization plugins); its `config()` and `about()` boxes are modal Win32 dialogs parented to `main_window`, requiring interactive GUI automation.
3. **Visualization Streaming**: Does not show vis frame buffering or spectrum FFT analysis (only a vis plugin exercises vis frames).
4. **Strict Version Acceptance Range (U-7)**: S09 leaves open whether a stock Winamp 2.95 host strictly rejected version flags (`0x0F00_0000`). Our host masks them to accept both 2.x and 5.x builds of `in_openmpt.dll`.

## 5. Open Questions (S09 / S10)

1. **U-7: Version acceptance range in Winamp 2.95**:
   Whether 2.95 strictly checked `version == 0x100` or tolerated upper flag bits. Our loader masks `IN_VERSION_FLAG_WIDE_FILENAMES` and `IN_VERSION_FLAG_INIT_STATUS` to allow both libopenmpt 2.x and 5.x builds.
2. **U-8: Role of General Plugin `init` return value**:
   Winamp 5.66 unloads on non-zero return, but 2.95 behavior is unverified.
3. **U-16: Nullable callback tolerance in InModule**:
   Which input callbacks may be null in 2.95.
4. **S10 §13.6: IPC 334 / 335 / 336 (INI and Plugin Directory Pointers)**:
   In-process pointer returns for settings paths have not been measured against a running 2.95 with an in-process plugin. Currently, unhandled IPC queries return `1` (the unsupported-id convention).
