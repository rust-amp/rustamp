# VIS — Plan: Shipping an Open-Source MilkDrop-Compatible Visualisation Plugin

**Status:** Decision-ready investigation and architecture plan. Open for owner review.  
**Date:** 2026-10-18  
**Owned path:** `docs/tasks/VIS-plugin-plan.md` only.  
**Boundary:** Write only inside this worktree (`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave86-milkdrop`). Never touch `D:\code\Rustamp`, `D:\code\rustamp-research`, or `D:\code\rustamp-refbin`. No code written, no dependency added, no download performed, no commit made.

---

## Executive Summary

The project owner wants an open-source MilkDrop-like visualisation bundled with Rustamp. Rustamp is a clean-room Winamp 2.95 reimplementation in Rust, with a split-licensing architecture (GPL-3.0-or-later player, MIT/Apache-2.0 plugin host) and strict clean-room legal boundaries (`CLEANROOM.md`).

This plan provides a decision-ready assessment of how to achieve this goal:

1. **What exists:** Five candidate projects are evaluated: MilkDrop 2.25c source (Nullsoft BSD-3), projectM (LGPL-2.1-or-later), butterchurn (MIT), Cockos WDL/EEL2 (zlib), and modern community forks.
2. **Project rules and obligations:** Shipping a bundled plugin imposes strict requirements on license compatibility, third-party notices, source offers, dynamic vs. static linking, clean-room header quarantine, and `xtask` package layout allowlists.
3. **Driving the visualisation:** Two technical paths exist: (A) a classic 32-bit Winamp `vis_` DLL driven by `rustamp-plughost32.exe` over the `specs/S09-plugin-abi.md` §5 ABI, and (B) a native 64-bit Rust/wgpu port (`rustamp-milkdrop`) consuming the audio engine's `VisTap`.
4. **The honest recommendation:** **Ship LATER (do not ship now).** Rustamp cannot ship a MilkDrop plugin today because:
   - The plugin host (`rustamp-plughost`) has no visualization loader or render loop yet (P-vis is not implemented).
   - The audio engine's visualization tap (`rustamp-audio::vistap`) outputs zero for spectrum data because spec S08 is unmeasured and open.
   - The native Rust port (`rustamp-milkdrop`) is a 9-line skeleton requiring months of shader and math-compiler engineering (scheduled for Milestone M4).
   - No clean-room compiled binary exists in the repository or approval store, and the package layout audit (`package.rs`) strictly forbids unapproved files.
   - Owner legal review and artifact download approvals are mandatory prerequisites under project rules.

---

## 1. What Exists: Open-Source and MilkDrop-Compatible Candidates

The table and detailed profiles below evaluate known open-source implementations. Because this analysis is conducted without network access, facts known with high confidence are distinguished from those requiring verification before owner reliance.

### Candidate Overview

| Candidate | Nature | Language / Stack | Declared License | Knowledge Confidence | Clean-Room Status in `CLEANROOM.md` |
|---|---|---|---|---|---|
| **MilkDrop 2.25c Source** (`vis_milk2`) | Original Winamp 2/5 visualization plugin | C++, Direct3D 9, HLSL, NS-EEL2 | Nullsoft BSD (3-Clause) | High on source; Medium on `ns-eel2` | Clean for `vis_milk2/` BSD files; **Dirty** for bundled SDK headers |
| **projectM** (`libprojectM`) | Reimplementation of MilkDrop 1 & 2 | C++ (C++17), OpenGL, GLSL | LGPL-2.1-or-later | High on core; Medium on wrapper DLLs | Reference only; **Do not copy** without separate review |
| **butterchurn** | WebGL2/WebAssembly port of MilkDrop 2 | JavaScript / TypeScript / WebGL2 | MIT License | High | Clean (copy allowed with attribution) |
| **Cockos WDL / EEL2** | Standalone expression evaluation engine | C / C++ | zlib License | High | Clean (copy allowed with attribution) |
| **Community Forks** (MilkDrop 2077, beatdrop, Spout-MilkDrop) | Modernized Direct3D 11/OpenGL forks | C++, Direct3D 11 / OpenGL | Mixed / BSD-3 / Unclear | Low-to-Medium | Unreviewed; likely contaminated by Winamp SDK headers |

---

### Detailed Candidate Profiles

#### 1. MilkDrop 2.25c Source (`vis_milk2`, geisswerks.com)
- **What it actually is:** The canonical desktop MilkDrop 2.25c visualization plugin (`vis_milk2.dll`) written by Ryan Geiss and released by Nullsoft/AOL. Implements preset evaluation, per-frame and per-vertex custom expressions, Direct3D 9 render targets, composite and warp HLSL pixel shaders, beat detection, and preset transitions.
- **Dependencies & Requirements:**
  - 32-bit x86 C++ toolchain (MSVC).
  - Legacy Direct3D 9 runtime and DirectX SDK / D3DX9 helper libraries (`d3dx9_*.dll`).
  - Win32 GDI/User32 message loop and window management.
  - Expression engine: bundled `ns-eel2/` (Nullsoft Expression Evaluation Library 2).
  - Preset library: `.milk` preset files, plus textures and documentation.
- **License:** Nullsoft BSD (3-clause) on files in `vis_milk2/` carrying the header.
- **Project Boundary & Clean-Room Trap:** `CLEANROOM.md` §1 line 17 explicitly warns:  
  > *"files under `vis_milk2/` that carry the Nullsoft BSD-3 header: copy allowed, with BSD notice kept and attribution... **Exception:** the bundled Winamp SDK headers in that zip (`Winamp/`, `Wasabi/`, `Agave/`, `vis_milk2/vis.h`, `api.h`) and any file without a BSD header are dirty. Check the `ns-eel2/` license separately before use."*
- **Facts to be verified by owner:**
  - *Must verify:* Exact license text and provenance of all files inside `ns-eel2/` within `milkdrop_225c_src.zip`.
  - *Must verify:* Clean-room compilability: can `vis_milk2` compile cleanly against Rustamp's clean-room ABI headers (from `specs/S09-plugin-abi.md`) with all dirty Winamp SDK headers completely excised?
  - *Must verify:* D3DX9 redistribution terms and whether modern Windows 10/11 requires the legacy DirectX 9 End-User Runtimes.

#### 2. projectM (`libprojectM`)
- **What it actually is:** An independent, cross-platform C++ reimplementation of MilkDrop preset visualization. Provides a shared C/C++ library (`libprojectM`) that compiles presets, parses EEL expressions, executes shaders via OpenGL / GLSL, and accepts PCM / FFT audio frames. Frontends exist for SDL, Qt, PulseAudio, and formerly a Winamp plugin wrapper.
- **Dependencies & Requirements:**
  - Desktop OpenGL (or OpenGLES) driver context.
  - Modern C++ runtime (MSVC C++17 runtime on Windows).
  - CMake build system.
  - Preset directory containing `.milk` presets and textures.
- **License:** LGPL-2.1-or-later.
- **Project Boundary & Clean-Room Status:** `CLEANROOM.md` §1 line 16 and §7 line 82 classify projectM as **"LGPL, reference only; do not copy from projectM without a separate review."**
- **Facts to be verified by owner:**
  - *Must verify:* Whether a modern classic Winamp plugin wrapper exists in upstream projectM v3/v4 or if one must be maintained.
  - *Must verify:* Submodule licensing (some projectM components have historically integrated various EEL implementations).
  - *Must verify:* Whether shipping an LGPL DLL satisfies the owner's legal stance on third-party licenses in the distribution.

#### 3. butterchurn (captbaritone)
- **What it actually is:** An open-source JavaScript / WebAssembly port of MilkDrop 2, developed by Jordan Eldredge (the author of Webamp). Transpiles `.milk` presets and EEL expressions into WebAssembly / JavaScript and compiles HLSL shaders to WebGL2 fragment shaders.
- **Dependencies & Requirements:**
  - Originally designed for modern web browsers (Node.js / WebAssembly / WebGL2 context).
  - To run in Rustamp: requires either embedding a headless JS/Wasm runtime (e.g. V8 / Wasmtime / QuickJS) or porting its transpiler and evaluation pipeline into native Rust.
- **License:** MIT License.
- **Project Boundary & Clean-Room Status:** Fully permissive; copy allowed with attribution (`CLEANROOM.md` §1 line 16).
- **Facts to be verified by owner:**
  - *Must verify:* Whether any independent native Rust port of butterchurn's parser and shader transpiler exists on crates.io (offline inspection confirms none is currently in `Cargo.lock`).
  - *Must verify:* Effort to port the butterchurn math pipeline to `wgpu` / WGSL.

#### 4. Cockos WDL / EEL2
- **What it actually is:** Expression Evaluation Library 2 from Cockos (Justin Frankel / REAPER). A fast, permissive, clean-room C expression compiler/evaluator that shares syntax and semantics with MilkDrop's per-frame and per-vertex expression code.
- **Dependencies & Requirements:** C/C++ compiler or Rust FFI bindings / port.
- **License:** zlib License (permissive).
- **Project Boundary & Clean-Room Status:** Fully permissive; listed in `CLEANROOM.md` §1 line 16.
- **Facts to be verified by owner:**
  - *Must verify:* Precision and compatibility differences between WDL-EEL2 and MilkDrop's original NS-EEL2 when rendering complex presets.

#### 5. Community Modernizations (MilkDrop 2077, beatdrop, Spout-MilkDrop)
- **What they are:** Independent GitHub forks attempting to update `vis_milk2` to Direct3D 11, Direct3D 12, or 64-bit x64 native architectures.
- **Dependencies & Requirements:** Modern Windows SDK, Direct3D 11/12 runtimes.
- **License & Provenance:** Highly variable; often uncredited or retaining proprietary Winamp SDK files.
- **Clean-room assessment:** High risk. Should not be touched without extensive provenance auditing.

---

## 2. Project Rules and Shipping Obligations

Shipping any bundled third-party or derivative visualization component must strictly comply with Rustamp's architecture and licensing rules:

### 2.1 The Split-License Model and Exception Limits

Per [RFC 0004](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave86-milkdrop/docs/rfc/0004-plugin-licensing.md) and [THIRD_PARTY.md](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave86-milkdrop/THIRD_PARTY.md):
- **Player & Internal Crates (`rustamp.exe`, `rustamp-vis`, `rustamp-milkdrop`):** `GPL-3.0-or-later`.
- **Plugin Host & Wire Crates (`rustamp-plughost32.exe`, `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-mediahost`):** `MIT OR Apache-2.0`.
- **GPL §7 Plugin Exception (`PLUGIN-EXCEPTION.md`):** Grants permission to combine the GPL player with independent modules communicating over `rustamp-plugin-abi` and `rustamp-ipc`.  
  **Crucial constraint:** As documented in `THIRD_PARTY.md` lines 52–56, the plugin exception covers **Rustamp's own code only**. It does *not* extend to third-party code. Third-party material must be compatible with the license of the crate it is linked into on its own terms.

### 2.2 License Compatibility by Integration Path

1. **Path A: Shipped as a 32-bit Classic Plugin DLL (`vis_milk2.dll` loaded by `rustamp-plughost32.exe`)**
   - The plugin host process is strictly permissive (`MIT OR Apache-2.0`).
   - Loading an external DLL through Win32 `LoadLibraryExW` inside `rustamp-plughost32.exe` introduces no copyleft taint onto either the host or the player.
   - If using **Nullsoft BSD-3 (`vis_milk2`)**: Permissive. Fully compatible.
   - If using **LGPL-2.1-or-later (`projectM`)**: Permissive host can load an LGPL DLL dynamically. Dynamic linking satisfies LGPL §6 (or §4 under LGPLv3), provided the user can replace the LGPL library.
   - If using **MIT (`butterchurn` derived C++ wrapper)**: Fully compatible.

2. **Path B: Shipped as Native Code inside `rustamp-milkdrop` (linked into `rustamp.exe`)**
   - The player is `GPL-3.0-or-later`.
   - **MIT** (butterchurn port) and **zlib** (WDL/EEL2) can be statically compiled into `rustamp-milkdrop` without conflict.
   - **Nullsoft BSD-3** (`vis_milk2` C++ code ported to Rust) can be incorporated provided BSD copyright notices and disclaimers are retained.
   - **LGPL-2.1-or-later** (`projectM`): Static compilation into `rustamp.exe` would require providing object files to allow relinking under LGPL terms. Dynamic linking via a separate shared library (`projectm.dll`) would be required to maintain clean boundaries.

### 2.3 Required Distribution Obligations

Regardless of candidate choice, the following legal and packaging obligations apply:
1. **Notice Preservation:**
   - Upstream copyright notices, author attributions, and full license texts must be retained in the source tree and listed in `THIRD_PARTY.md`.
   - Shipped packages must contain notice files (e.g. `NOTICES.txt` / `THIRD_PARTY.md`) alongside the binaries, as identified in `RELEASE-plan.md` item G1.
2. **Source Offer Obligations:**
   - For **GPL-3.0** components (`rustamp.exe` and any GPL modifications): Must provide complete corresponding source or a written offer.
   - For **LGPL-2.1** components (`projectM`): Must provide the library's corresponding source code and build instructions so the end user can modify and relink the library.
   - For **BSD-3 / MIT**: Must reproduce the copyright notice, license conditions, and disclaimer in the documentation and package notices.
3. **Dynamic vs. Static Linking:**
   - Classic plugin DLLs loaded by `rustamp-plughost32.exe` are dynamically loaded at runtime via OS APIs (`LoadLibraryExW`).
   - Native Rust crates are statically linked into the player workspace.

### 2.4 Clean-Room Boundary Enforcement

`CLEANROOM.md` imposes binding legal rules:
- **Zero Exposure to Dirty Headers:** Implementers must never read Winamp SDK headers (`vis.h`, `winamp/`, `wasabi/`) or disassembly.
- **The Quarantine of `milkdrop_225c_src.zip`:** If compiling `vis_milk2` from Nullsoft BSD source:
  - The dirty SDK headers bundled in the archive (`vis_milk2/vis.h`, `Winamp/`, etc.) must be deleted or quarantined.
  - Compilation must link against clean-room ABI definitions (the neutral facts audited in `specs/S09-plugin-abi.md` and exported as a clean C header or Rust `cdylib` wrapper).
- **Explicit Artifact Approval:** Under `CONTRIBUTING.md` and `P3-dll-acceptance.md`, any reference binary or third-party DLL requires **prior owner approval**. Automated downloads are strictly forbidden. The plugin host enforces an approval store (`approval.rs`) that verifies the exact SHA-256 hash of any loaded DLL before execution.

### 2.5 Package Layout & Audit Restrictions

The distribution packager (`xtask/src/package.rs`) enforces strict directory allowlists via `audit_directory` and `audit_package_layout`:
- Currently, the package layout only allows:
  - `player/`: `rustamp.exe`, `rustamp-dechost-symphonia.exe`, `rustamp-dsphost-stages.exe`, `LICENSE`, `PLUGIN-EXCEPTION.md`, `manifest.json`, `QUICKSTART.md`, and optionally `rustamp-plughost32.exe`.
  - `host/`: `rustamp-plughost32.exe`, `LICENSE-MIT`, `LICENSE-APACHE`, `manifest.json`.
- **Packaging Impact:** To ship a bundled plugin (e.g. `Plugins/vis_milk2.dll` and preset files):
  1. The layout audit in `xtask/src/package.rs` must be edited and approved to allow a `plugins/` directory and preset assets.
  2. The package manifest generator (`serde_manifest_player`) must record the new files.
  3. The approval store must include the pinned SHA-256 hash of the bundled DLL.

---

## 3. Driving the Visualisation: Architecture and Audio Pipeline

Rustamp has two potential driving architectures for visualization:

```
[Audio Engine / Decode Thread]
         │
         ▼
  [VisTap (vistap.rs)]  ──(neutral VisFrame: 576 waveform samples f32 [-1,1], spectrum)──┐
                                                                                           │
  ┌────────────────────────────────────────────────────────────────────────────────────────┘
  │
  ├──► PATH A: Classic Plugin Path (32-bit out-of-process)
  │      │
  │      ▼ (IPC / Shared Memory Ring)
  │    [rustamp-plughost32.exe]
  │      │  (Convert f32 to Winamp 8-bit waveform and 0..255 spectrum per spec S08)
  │      ▼
  │    [VisModule (0x938 bytes)]
  │      ├─ waveform_data: [[u8; 576]; 2]
  │      ├─ spectrum_data: [[u8; 576]; 2]
  │      ▼
  │    [vis_milk2.dll] ──► renders via Direct3D 9 into Win32 Window
  │
  └──► PATH B: Native Rustamp Path (64-bit in-process)
         │
         ▼
       [rustamp-milkdrop (wgpu)]
         ├─ EEL2 Expression Evaluator
         ├─ Naga / Shaderc WGSL compilation
         ▼
       Renders via modern GPU API (D3D12/Vulkan) into player viewport
```

---

### 3.1 Path A: Classic 32-bit Plugin via `rustamp-plughost32.exe`

This path implements the standard Winamp 2.x visualization plugin architecture defined by `specs/S09-plugin-abi.md` §5.

#### The ABI Contract (S09 §5)
- **Exported Entry Point:** `winampVisGetHeader()` (undecorated `cdecl`, returns `*mut VisHeader`).
- **Header (`VisHeader`):** 12 bytes (`version` = `0x101`, `description`, `get_module(index)`).
- **Module (`VisModule`):** 2360 bytes (`0x938`):
  - Host writes: `parent_window` (HWND), `dll_instance` (HMODULE), `sample_rate`, `channels`.
  - Plugin writes: `latency_ms` (0 for MilkDrop), `delay_ms` (10 ms for MilkDrop), `spectrum_rows` (2), `waveform_rows` (2), and callbacks: `config(module)`, `init(module) -> int`, `render(module) -> int`, `quit(module)`.
  - Buffer arrays: `spectrum_data` (`2 x 576` unsigned bytes), `waveform_data` (`2 x 576` unsigned bytes).

#### The Driving Loop
- `rustamp-plughost32.exe` must spawn a dedicated **vis thread** (S09 §5.3):
  1. Call `init(module)`; abort if non-zero.
  2. Loop at interval `render_time + delay_ms`:
     - Pump messages for the plugin window.
     - Fetch current `VisFrame` corresponding to `playback_position + latency_ms`.
     - Quantize and copy audio data into `VisModule.waveform_data` and `spectrum_data`.
     - Call `render(module)`.
  3. On stop or exit: call `quit(module)`, free buffers, unload DLL.

#### Audio Tap Feeding and the S08 Blocker
- The engine's tap (`crates/rustamp-audio/src/vistap.rs`) produces `VisFrame`s containing 576 samples of waveform as `f32` in `[-1.0, 1.0]`.
- **The S08 Blocker:** In `vistap.rs` lines 13–16:  
  > *"`spectrum` — left at zero. The FFT, its window, the bin mapping and the 8-bit scaling are spec **S08**; guessing them here would bake an unverified derivation into the engine."*
- Classic MilkDrop relies heavily on the 576-byte spectrum and waveform arrays for beat detection, energy estimation, and waveform rendering. **Without spec S08 being measured and ratified, feeding `VisModule` with current engine data delivers zeros for all frequency bands.**
- Furthermore, the waveform sign/scaling convention (signed 8-bit two's complement vs. unsigned 8-bit offset by 128) remains marked `[UNV]` in S09 §5.2.

#### Current Plughost Implementation Status
- `rustamp-plughost` currently has active bridges for `InModule` (input plugins), `DspModule` (task P6), and `GeneralPlugin` (task P3).
- **`VisModule` has NO loader, no thread loop, and no IPC bridge in `rustamp-plughost` today.** Implementing this bridge is an unstarted engineering task (the "P-vis" milestone).

---

### 3.2 Path B: Native 64-bit Rust Visualisation (`rustamp-milkdrop`)

This path follows Milestone M4 in `docs/PLAN.md` §2/§5:
- Native 64-bit implementation in `crates/rustamp-milkdrop`.
- Consumes `VisFrame` directly from the audio engine across an in-process lock-free channel (zero IPC overhead, 64-bit native).
- Graphics stack: `wgpu` presenting into a `softbuffer` or `winit` surface.
- **Components required:**
  1. `.milk` preset parser (parsing ini-like MilkDrop 2 preset files).
  2. EEL2 expression compiler and VM (translating per-frame and per-vertex formulas to bytecode or Cranelift JIT).
  3. Shader transpiler: MilkDrop presets use HLSL pixel shaders (`warp` and `comp`). They must be transpiled from HLSL to SPIR-V via `shaderc` and then to WGSL via `naga`.
  4. Rendering pipeline: Multi-pass render-to-texture ping-pong buffers, motion vectors, and composite blending.
- **Current Status:** `crates/rustamp-milkdrop/src/lib.rs` is an empty 9-line skeleton. Implementing this from scratch represents an estimated 4 to 8 engineer-weeks of specialized compiler and graphics programming.

---

### 3.3 Path C: Dedicated Visualization Resident (`rustamp-vishost32.exe`)

An alternative evolutionary path:
- Modeled after `rustamp-dechost` and `rustamp-dsphost` (RFC 0006 resident model).
- Rather than overloading `rustamp-plughost32.exe`, a dedicated 32-bit helper (`rustamp-vishost32.exe`) is spawned solely to host legacy Direct3D 9 visualization plugins.
- Isolates Direct3D 9 device crashes, DirectX runtimes, and Win32 message pumping from both the player and the audio decoder host.

---

## 4. The Honest Recommendation, Costs, and Verification Roadmap

### 4.1 Recommendation: Ship LATER (Do Not Ship Now)

**Rustamp should NOT attempt to ship a MilkDrop plugin today.**

#### Why "Ship Now" is Impossible:
1. **The Plugin Host is Not Ready:** `rustamp-plughost` has no visualization plugin loader or render thread implementation. It cannot load `vis_*.dll` files.
2. **Audio Data Tap is Incomplete:** Spec S08 is unmeasured. Spectrum data is completely absent (all zeros), and waveform scaling is unverified. MilkDrop running on empty spectrum data produces flat, unresponsive visuals.
3. **Native Port is Unwritten:** `rustamp-milkdrop` is a milestone M0 skeleton.
4. **No Approved Binary Exists:** The project has strict clean-room rules forbidding arbitrary binary inclusion without explicit owner approval. No compiled `vis_milk2.dll` exists in the tree or the approval store.
5. **Packager Rejection:** `xtask/src/package.rs` enforces strict allowlists and will reject any build containing unlisted plugin files.
6. **Clean-Room & Legal Gates Unresolved:** The clean-room separation of `vis_milk2` from dirty Winamp SDK headers has not been audited or ratified by the owner.

---

### 4.2 Cost and Comparison Matrix

| Option | Engineering Cost | Clean-Room & Legal Risk | Runtime Dependencies | Recommended Milestone |
|---|---|---|---|---|
| **Option 1: Classic `vis_milk2.dll` via Plughost** | **Medium (2–3 weeks)**<br>(Implement P-vis loader in plughost + S08 FFT derivation) | **Low-Medium**<br>(Requires clean C header; quarantine dirty SDK headers) | Legacy Direct3D 9, D3DX9 runtime, WOW64 on x64 | **Milestone M3** (Classic Plugins) |
| **Option 2: Native `rustamp-milkdrop` (wgpu)** | **High (6–10 weeks)**<br>(EEL2 engine + HLSL→WGSL pipeline + preset parser) | **Zero**<br>(Pure clean-room Rust code derived from MIT butterchurn / zlib EEL2) | Modern GPU (DirectX 12 / Vulkan / Metal), 64-bit native, fully cross-platform | **Milestone M4** (Modern Underneath) |
| **Option 3: projectM integration** | **Medium-High (3–4 weeks)**<br>(C FFI wrapper + OpenGL context integration) | **Medium**<br>(LGPL source offer and dynamic relinking obligations) | OpenGL 3.3+, C++ runtime | Not recommended (conflicts with Winamp 2.95 fidelity) |

---

### 4.3 What Must Be Verified First (The Owner Gate)

Before the project can move forward with any MilkDrop implementation, the owner must complete and approve the following items:

1. **Owner Legal Review of MilkDrop 2.25c Source:**
   - Confirm that `vis_milk2/` files carrying the Nullsoft BSD-3 header can be compiled using a clean C header derived from `specs/S09-plugin-abi.md`, with all dirty files (`Winamp/`, `Wasabi/`, `vis.h`, `api.h`) stripped.
   - Verify the license terms of the bundled `ns-eel2/` expression evaluator.
2. **Owner Decision on Binary Artifact Approval & Download:**
   - Decide whether Rustamp will distribute a pre-compiled `vis_milk2.dll` binary (requiring explicit owner download approval and hash pinning in `approval.rs`) or provide build scripts in `xtask` for the user to compile it.
   - *Reminder:* Project rules forbid agents from downloading reference binaries without explicit user consent.
3. **Dirty-Room Ratification of Spec S08:**
   - Authorize and execute the S08 probe plugin measurements in the test VM (per `docs/tasks/S08-vis.md`) to measure real FFT bin mapping, magnitude scaling, and waveform conventions so the audio tap produces accurate spectrum data.
4. **Package Layout & Notices Update:**
   - Authorize additions to `THIRD_PARTY.md` and package notices.
   - Approve edits to `xtask/src/package.rs` to allow shipping a `plugins/` directory and presets.

---

### 4.4 Phased Implementation Roadmap

When the owner decides to proceed, work should follow this sequence:

```
[Phase 1: Spec S08 Ratification]
  └─ Dirty-room probe measures Winamp 2.95 spectrum FFT & waveform scaling.
  └─ Audit and export `specs/S08-vis.md`.
  └─ Implement FFT and bin mapping in `rustamp-audio::vistap`.

[Phase 2: Task P-vis (Plugin Host Vis Bridge)]
  └─ Implement `winampVisGetHeader` resolution in `rustamp-plughost`.
  └─ Add `VisModule` lifecycle: vis thread, render loop, delay_ms timing.
  └─ Extend IPC wire types in `rustamp-ipc` to stream `VisFrame` to plughost.

[Phase 3: Clean-Room Compilation & Packaging of vis_milk2]
  └─ Owner reviews and approves clean `vis_milk2` source build against clean headers.
  └─ Register approved DLL hash in `rustamp-plughost::approval`.
  └─ Update `xtask/src/package.rs` allowlists to permit `plugins/vis_milk2.dll`.

[Phase 4: Milestone M4 — Native rustamp-milkdrop]
  └─ Clean-room Rust port of butterchurn parser and EEL2 on `wgpu`.
  └─ Native 64-bit cross-platform visualization without legacy 32-bit x86 or D3D9.
```

---

## 5. Verification Checklist & Boundary Receipt

### Worktree Boundary
- [x] Only `docs/tasks/VIS-plugin-plan.md` was created or modified.
- [x] Paths outside the worktree (`D:\code\Rustamp`, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`) were never read or written.
- [x] No code was changed in any crate (`crates/*`).
- [x] No dependencies were added to `Cargo.toml`.
- [x] No network access or downloads were performed.
- [x] No commits were made (`git status --short` verified).
- [x] No cargo tests or VM sessions were run.
