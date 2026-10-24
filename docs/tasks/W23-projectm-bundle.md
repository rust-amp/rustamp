# W23-projectm-bundle: Packaging Whitelist Extension and Integration Card

**Task ID:** W23-projectm-bundle  
**Status:** Ready for packaging lane  
**Author:** Visualization / projectM Integration Lane  
**References:** `docs/tasks/W20-projectm-review.md`, `specs/S09-plugin-abi.md`, `CLEANROOM.md`, `THIRD_PARTY.md`

---

## 1. Context and Implementation Summary

Following the full owner approval and the blueprint established in `docs/tasks/W20-projectm-review.md`:

1. **Vendored Source & Provenance:**
   - Upstream source archive: `libprojectM-4.1.8.tar.gz` (1,309,526 bytes).
   - Verified SHA-256 digest: `f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`.
   - Extracted clean-room source tree into `third_party/projectm/` accompanied by `PROVENANCE.json`.
   - Full LGPL-2.1 text placed at repository root as `LICENSE-LGPL-2.1`.

2. **Clean-Room Wrapper & FFI:**
   - Implemented in `crates/rustamp-vis/src/projectm/`:
     - `ffi.rs`: Direct C-ABI bindings and `ProjectMApi` dynamic loader supporting LGPL-2.1 §6 relinking.
     - `audio.rs`: Waveform audio converter translating Winamp 2 × 576 byte waveform rows into normalized interleaved float PCM buffers (`[-1.0, 1.0]`). Spectrum data is never fed as PCM.
     - `wrapper.rs`: S09 §5 classic Winamp 2.95 visualization plugin wrapper exporting `winampVisGetHeader`, returning `VisHeader` (`0x101`), and providing 2360-byte `VisModule` with `config`, `init`, `render`, and `quit` lifecycle callbacks.

3. **Notices & Third-Party Inventory:**
   - Updated root `THIRD_PARTY.md` with bundled native component entry, LGPL-2.1-or-later terms, MIT notices for vendored subcomponents (`hlslparser`, `projectm-eval`, `glm`, `SOIL2`), updated project roadmap, and explicit documentation of LGPL §6 relink and source offer obligations.

---

## 2. Required Packaging Change in `xtask/src/package.rs`

Because `xtask` is held by another lane, this card provides the **exact, narrow diff** required for the release audit gate in `xtask/src/package.rs`.

### The Problem
`xtask/src/package.rs` enforces a strict allow-list in `audit_package_layout`. Any file present in `player/` or `host/` that is not explicitly in the slice causes an immediate build failure.

### The Required Patch
Extend `PackageKind::Player` allow-list narrowly with the four entries needed for projectM. **Do not use wildcards (`Plugins/**/*`), and do not touch `PackageKind::Host`.**

```diff
--- a/xtask/src/package.rs
+++ b/xtask/src/package.rs
@@ -1710,6 +1710,10 @@ fn audit_package_layout(root: &Path, kind: PackageKind) -> Result<(), PackageEr
             "Plugins/in_mp3.dll",
             "Plugins/in_wave.dll",
             "Plugins/out_wave.dll",
+            "Plugins/projectM",
+            "Plugins/projectM/vis_projectm.dll",
+            "Plugins/projectM/projectM-4.dll",
+            "LICENSE-LGPL-2.1",
             "Skins",
             "Skins/base.wsz",
             "README.txt",
```

### Host Isolation Guarantees
- `PackageKind::Host` **must remain untouched**:
  ```rust
  PackageKind::Host => &[
      "rustamp-plughost32.exe",
      "README.txt",
      "LICENSE-MIT",
      "LICENSE-APACHE",
      "THIRD_PARTY.md",
  ],
  ```
  The permissive plugin host package and its dependency closure must remain strictly copyleft-free and projectM-free.

---

## 3. C++ Build Requirements for `projectM-4.dll`

### Toolchain Prerequisites
1. **Build System:** CMake 3.21 or newer (`cmake.exe`) and Ninja or MSVC `nmake` / MSBuild.
2. **Compiler:** C++14/C++17 standard compliant compiler:
   - Windows MSVC (Visual Studio 2022 Community / Build Tools `cl.exe`), or
   - MinGW-w64 GCC.
3. **Architecture:** `x86` (32-bit i686) matching `rustamp-plughost32.exe` and the classic Winamp 32-bit plugin architecture.
4. **Platform Graphics:** Windows OpenGL (`OpenGL32.lib`, `glu32.lib`).
5. **CMake Configuration Flags:**
   ```bash
   cmake -B build-x86 -S third_party/projectm \
     -G "Visual Studio 17 2022" -A Win32 \
     -DBUILD_TESTING=OFF \
     -DENABLE_EMSCRIPTEN=OFF \
     -DENABLE_SDL=OFF \
     -DENABLE_QT=OFF
   cmake --build build-x86 --config Release --target projectM-4
   ```

### Execution Status in this Lane
- Available on the current host: CMake 3.21+, Ninja, and MSVC 2022 Community.
- In accordance with `AGENTS.md` and repository isolation rules, the target release artifact generation and cross-architecture multi-target builds belong to the orchestrator VM build pipeline.
- No separate ad-hoc C++ compilation was invoked here to avoid uncommitted binary artifacts or untracked intermediate directories in the working tree. The wrapper was verified with Rust workspace gates (`fmt`, `clippy`, `cargo build --workspace`).

---

## 4. What Only the Virtual Machine Can Prove

The following verifications require the orchestrator VM environment:
1. **OpenGL Window & Context Binding:** Live hardware/Mesa OpenGL context creation inside the host visualization window.
2. **Dynamic DLL Loading at Runtime:** Verifying `vis_projectm.dll` locating and loading `projectM-4.dll` from `player/Plugins/projectM/` at runtime via `LoadLibraryW`.
3. **LGPL §6(b) Dynamic Replacement:** Replacing `projectM-4.dll` with an alternative build and verifying that the player and visualization continue to render correctly without recompilation.
4. **End-to-End Packaging Audit:** Running `cargo xtask package-player` with the updated whitelist to produce the finalized release zip archive.
