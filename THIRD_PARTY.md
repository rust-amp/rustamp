# Third-party code and assets

Rustamp is licensed under GPL-3.0-or-later, except for the five permissive crates
(see "Licensing of Rustamp itself" below). Every piece of code or asset taken
from another project must be listed here, and its original license header must be
kept in place (see [CLEANROOM.md](CLEANROOM.md)).

| Component | Source | License | Used in | Notes |
|---|---|---|---|---|
| webamp classic-skin tables | [captbaritone/webamp](https://github.com/captbaritone/webamp), commit `79117c66d79fdac690fc732a0ce8df543ccd97a0` | MIT | `crates/rustamp-skin/src/webamp.rs` | The sprite rectangles (`packages/webamp/js/skinSprites.ts`), the `text.bmp` glyph grid, the cursor name list, the default analyser colours and the default playlist colours (`packages/webamp/js/baseSkin.json`, `constants.ts`), rewritten in Rust. The MIT notice is at the top of the module. Every ported value carries a `// provisional: webamp, pending S01/S17` marker. |
| libprojectM 4.1.8 | [projectM-visualizer/projectm](https://github.com/projectM-visualizer/projectm/releases/tag/v4.1.8), release `v4.1.8` (`libprojectM-4.1.8.tar.gz`, SHA-256 `f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`, 1,309,526 bytes) | LGPL-2.1-or-later (see [LICENSE-LGPL-2.1](LICENSE-LGPL-2.1)), with vendored subcomponents under MIT: `hlslparser` (MIT), `projectm-eval` (MIT), `glm` (Happy Bunny / MIT), `SOIL2` (Public Domain / MIT) | `third_party/projectm`, wrapped by `crates/rustamp-vis` (`vis_projectm.dll`), deployed to `player/Plugins/projectM/` (player package only; never enters permissive `host/` package or its dependency closure) | Maintained C++ library for MilkDrop preset visualization. Wrapped via original clean-room classic `vis_` FFI wrapper in `rustamp-vis`. Fulfills LGPL-2.1 Section 6 duties: dynamic linking with replaceable shared library (`projectM-4.dll`), source offer covering all Section 6 materials, and relink capability. |

## Rust crate dependencies

Crates linked as ordinary Cargo dependencies (no code is copied into the
tree). The license is the one each crate declares in its published
`Cargo.toml`, read from the local cargo registry and cross-checked with
`cargo tree --format "{p} {l}"`; every one is permissive and compatible with
GPL-3.0-or-later. None of them is in the dependency closure of a permissive
Rustamp crate: they enter through `rustamp-audio`, which is GPL-3.0-or-later.

Projects we expect to draw from: DeaDBeeF (zlib / GPL / LGPL), webamp (MIT), butterchurn (MIT; port source for rustamp-milkdrop), WDL/EEL2 (zlib), libopenmpt (BSD-3-Clause), projectM 4.1.8 (LGPL-2.1-or-later; selected native library bundled player-only with classic wrapper in rustamp-vis).


Not bundled: the original Winamp base skin. It is distributed separately as an optional download, and it is used in tests.

## Licensing of Rustamp itself

RFC 0004 splits the license. The player keeps its copyleft; the crates that
face plugins are permissive, so that no copyleft reaches a plugin of any
license, and a GPL-3.0 section 7 additional permission covers the player's own
code on the player ↔ plugin-host edge.

| Part | Crates | License |
|---|---|---|
| The player and every other current workspace member | `rustamp-core`, `rustamp-audio`, `rustamp-decoders`, `rustamp-dsp`, `rustamp-output`, `rustamp-meta`, `rustamp-playlist`, `rustamp-skin`, `rustamp-gfx`, `rustamp-ui`, `rustamp-vis`, `rustamp-milkdrop`, `rustamp-platform`, `rustamp-prefs`, `rustamp-library`, `rustamp-app`, `rustamp-testkit`, `xtask` | GPL-3.0-or-later (see [LICENSE](LICENSE)) |
| Plugin-facing and media-host protocol framework | `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost` | MIT OR Apache-2.0 (see [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)) |

All 23 current workspace members are version `0.0.1`. Per-resident helper
crates are not yet workspace members; they will carry the resident's license,
not automatically the framework's permissive license (RFC 0006 §8).

`rustamp-plughost32.exe` is the process that loads classic plugin DLLs. It links
only the permissive crates above, so the plugin edge carries no copyleft
whatsoever — for plugins of any license, including GPL-2.0-only ones, which are
incompatible with GPL-3.0.

**Section 7 plugin exception.** [PLUGIN-EXCEPTION.md](PLUGIN-EXCEPTION.md)
records the GPL-3.0 section 7 additional permission for the GPL-3.0-or-later
crates that contain only Rustamp's own code: it permits combining the covered
work with independent modules that communicate solely through the documented
classic-plugin ABI (`rustamp-plugin-abi`) and the documented IPC protocol
(`rustamp-ipc`, protocol version 1), regardless of those modules' licenses, and
requires the interfaces to stay unmodified. It does **not** apply to
third-party material. Its scope deliberately excludes `rustamp-decoders` and
`rustamp-skin`; `xtask` is not listed in that scope either. Linking third-party
GPL material cannot extend our permission to it. Review dependency changes
against the scope, not just against GPL compatibility.

**No copied third-party source in the permissive crates is recorded here.**
Nothing in the copied-source table above lives in the five permissive crates.
This does **not** mean they have no third-party dependencies: the Cargo
inventory below includes their serialization, error and Windows dependencies.
Their license
texts are the canonical MIT and Apache-2.0 texts, copied unmodified from a crate
in the local cargo registry.

## Native bundled components and LGPL-2.1 Section 6 compliance

### libprojectM 4.1.8

- **Component**: `libprojectM` 4.1.8
- **Upstream source**: [projectM-visualizer/projectm](https://github.com/projectM-visualizer/projectm), release `v4.1.8`
- **Release archive**: `libprojectM-4.1.8.tar.gz` (1,309,526 bytes, SHA-256 `f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`)
- **Vendored location**: `third_party/projectm/` (with `PROVENANCE.json`)
- **License**: LGPL-2.1-or-later. Full license text is available at [LICENSE-LGPL-2.1](LICENSE-LGPL-2.1) in the repository root.
- **Copyright holders**: Copyright (C) 2003-2023 projectM Team (Carmelo Piccione, Peter Sperl, Mischa Spiegelmock, Kyle Davis, and contributors).
- **Subcomponents (vendored)**:
  - `vendor/hlslparser`: MIT, Copyright (c) 2013-2023 Unknown Worlds Entertainment, Inc.
  - `vendor/projectm-eval`: MIT, Copyright (c) 2023 The projectM Team.
  - `vendor/glm`: MIT / Happy Bunny, Copyright (c) 2005-2023 G-Truc Creation.
  - `vendor/SOIL2`: Public Domain / MIT, Copyright (c) 2007-2023 Spartan Code Studios.
- **Packaging layout and host isolation**:
  - Bundled exclusively in the player distribution under `player/Plugins/projectM/` (`vis_projectm.dll`, `projectM-4.dll`, `LICENSE-LGPL-2.1`).
  - **Strict host isolation**: projectM and its LGPL license NEVER enter `host/`, `host/THIRD_PARTY.md`, or the permissive plugin-host dependency closure.
- **LGPL-2.1 Section 6 duties (relink capability and source offer)**:
  - **Section 6(b) Dynamic linking and relinking**: `vis_projectm.dll` dynamically links against `projectM-4.dll`. Users are entitled and able to replace `projectM-4.dll` with an interface-compatible custom or newer build without modifying or recompiling the host player or wrapper.
  - **Section 6(d) / Section 6(c) Source offer**: Rustamp provides a complete, machine-readable copy of the corresponding source code for `libprojectM 4.1.8` at release SHA-256 `f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`, including all build scripts, CMake configurations, and the clean-room wrapper source in `crates/rustamp-vis`.
  - **Reverse engineering & debugging**: Modification and reverse engineering of the library for debugging such modifications are explicitly permitted under the terms of LGPL-2.1.


## Cargo dependencies: purposes and consumers

Versions and exact declared licenses (including every transitive version) are
in the inventory below. These are dependencies, not source ports. Keep upstream
license/copyright notices when distributing; this metadata inventory is not a
replacement for license texts, source-offer obligations or an artifact review.

| Direct dependency | Purpose | Direct workspace consumers (including dev dependencies) |
|---|---|---|
| `thiserror` | Error definitions | core, gfx, ipc, mediahost, platform, skin, ui |
| `serde` | Serializable state and wire types | app, core, ipc, plugin-abi, proto |
| `postcard` | Wire serialization | ipc; proto (dev) |
| `crossbeam-channel` | Cross-thread channels | core, ipc |
| `toml` | Config serialization and I/O | app; core (dev) |
| `png` | Golden-image I/O | testkit |
| `tempfile` | Temporary files: the temporary file of the atomic tag write, and the test files of the crates below | meta; app, audio, decoders, skin, testkit (dev) |
| `windows` | Windows API bindings | ipc, platform, plughost, xtask (Windows only) |
| `rtrb` | Lock-free audio ring | audio |
| `cpal` | Cross-platform audio output; loopback capture | output; app (dev) |
| `rubato` | Band-limited sample-rate conversion (FFT engine; sinc fallback) | audio |
| `wasapi` | Windows audio output | output |
| `zip` | Skin archive reading | skin |
| `lofty` | Audio tags and metadata | meta |
| `winit` | Window/event-loop integration | app, ui |
| `softbuffer` | CPU-rendered window presentation | app, ui |
| `fontdb` | Headless discovery of the installed family requested by `pledit.txt` (`fs` feature only, no mmap/fontconfig) | ui |
| `ab_glyph` | Safe CPU rasterization of playlist system-font glyphs | ui |
| `embed-manifest` | Build-time: embeds the Windows application manifest (Common Controls 6 for the native preferences dialog) | app (build script) |
| `symphonia` | Audio decoding/demuxing | decoders; MPL-2.0, not permissive |
| `hound` | WAV test fixtures | decoders (dev) |
| `rusqlite` | SQLite-backed media library index | library |

Consumer names omit the `rustamp-` prefix, except `xtask`. Other entries below
are transitives supporting these dependencies, including non-Windows targets,
build tools and procedural macros. `rubato` is only a proposed dependency: it
is not in this lockfile and is deliberately not recorded as already used.

## Complete Cargo.lock license inventory

Snapshot: 2026-10-19, **313 external package versions**, all from crates.io.
`Cargo.lock` records names/versions/sources/checksums, **not licenses**; licenses
below are the exact cached package-manifest declarations returned by
`cargo metadata --offline --locked --all-features --format-version 1`.
Legacy `/` means alternative licensing; do not replace `AND` with `OR`.
The Symphonia family is MPL-2.0; `r-efi` has an LGPL alternative but also MIT
and Apache alternatives. Neither is in a protected Windows normal closure.

Regenerate the marked block with
`python .github/scripts/license-audit.py --inventory`, review the lock/manifest
delta and update the purpose table above. The default audit compares every
row, including licenses and sources, and rejects stale/missing/extra rows.
It checks permissive licensing only in the protected normal closures, not
as a blanket permissive requirement for the GPL player or other platforms.

<!-- BEGIN CARGO LICENSE INVENTORY -->
| Package | Version | Declared license | Source |
|---|---|---|---|
| `ab_glyph` | `0.2.32` | `Apache-2.0` | `crates.io` |
| `ab_glyph_rasterizer` | `0.1.10` | `Apache-2.0` | `crates.io` |
| `adler2` | `2.0.1` | `0BSD OR MIT OR Apache-2.0` | `crates.io` |
| `ahash` | `0.8.12` | `MIT OR Apache-2.0` | `crates.io` |
| `alsa` | `0.11.0` | `Apache-2.0/MIT` | `crates.io` |
| `alsa-sys` | `0.4.0` | `MIT` | `crates.io` |
| `android-activity` | `0.6.1` | `MIT OR Apache-2.0` | `crates.io` |
| `android-properties` | `0.2.2` | `MIT` | `crates.io` |
| `arrayref` | `0.3.9` | `BSD-2-Clause` | `crates.io` |
| `arrayvec` | `0.7.8` | `MIT OR Apache-2.0` | `crates.io` |
| `as-raw-xcb-connection` | `1.0.1` | `MIT OR Apache-2.0` | `crates.io` |
| `atomic-polyfill` | `1.0.3` | `MIT OR Apache-2.0` | `crates.io` |
| `atomic-waker` | `1.1.2` | `Apache-2.0 OR MIT` | `crates.io` |
| `audio-codec-algorithms` | `0.8.1` | `0BSD OR Apache-2.0` | `crates.io` |
| `audioadapter` | `5.0.0` | `MIT OR Apache-2.0` | `crates.io` |
| `audioadapter-buffers` | `5.2.0` | `MIT OR Apache-2.0` | `crates.io` |
| `audioadapter-sample` | `5.2.0` | `MIT OR Apache-2.0` | `crates.io` |
| `autocfg` | `1.5.1` | `Apache-2.0 OR MIT` | `crates.io` |
| `bitflags` | `1.3.2` | `MIT/Apache-2.0` | `crates.io` |
| `bitflags` | `2.13.2` | `MIT OR Apache-2.0` | `crates.io` |
| `block2` | `0.5.1` | `MIT` | `crates.io` |
| `block2` | `0.6.2` | `MIT` | `crates.io` |
| `bumpalo` | `3.20.3` | `MIT OR Apache-2.0` | `crates.io` |
| `bytemuck` | `1.25.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `bytemuck_derive` | `1.12.1` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `byteorder` | `1.5.0` | `Unlicense OR MIT` | `crates.io` |
| `bytes` | `1.12.1` | `MIT` | `crates.io` |
| `calloop` | `0.13.0` | `MIT` | `crates.io` |
| `calloop-wayland-source` | `0.3.0` | `MIT` | `crates.io` |
| `cc` | `1.8.0` | `MIT OR Apache-2.0` | `crates.io` |
| `cfg-if` | `1.0.5` | `MIT OR Apache-2.0` | `crates.io` |
| `cfg_aliases` | `0.2.2` | `MIT` | `crates.io` |
| `cobs` | `0.3.0` | `MIT OR Apache-2.0` | `crates.io` |
| `combine` | `4.6.8` | `MIT` | `crates.io` |
| `concurrent-queue` | `2.5.0` | `Apache-2.0 OR MIT` | `crates.io` |
| `core-foundation` | `0.9.4` | `MIT OR Apache-2.0` | `crates.io` |
| `core-foundation-sys` | `0.8.7` | `MIT OR Apache-2.0` | `crates.io` |
| `core-graphics` | `0.23.2` | `MIT OR Apache-2.0` | `crates.io` |
| `core-graphics-types` | `0.1.3` | `MIT OR Apache-2.0` | `crates.io` |
| `core_detect` | `1.0.0` | `MIT/Apache-2.0` | `crates.io` |
| `coreaudio-rs` | `0.14.2` | `MIT/Apache-2.0` | `crates.io` |
| `cpal` | `0.18.2` | `Apache-2.0` | `crates.io` |
| `crc32fast` | `1.5.2` | `MIT OR Apache-2.0` | `crates.io` |
| `critical-section` | `1.2.0` | `MIT OR Apache-2.0` | `crates.io` |
| `crossbeam-channel` | `0.5.17` | `MIT OR Apache-2.0` | `crates.io` |
| `crossbeam-utils` | `0.8.23` | `MIT OR Apache-2.0` | `crates.io` |
| `ctor` | `0.10.1` | `Apache-2.0 OR MIT` | `crates.io` |
| `cursor-icon` | `1.2.0` | `MIT OR Apache-2.0 OR Zlib` | `crates.io` |
| `dasp_sample` | `0.11.0` | `MIT OR Apache-2.0` | `crates.io` |
| `data-encoding` | `2.11.1` | `MIT` | `crates.io` |
| `dispatch` | `0.2.0` | `MIT` | `crates.io` |
| `dispatch2` | `0.3.1` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `dlib` | `0.5.3` | `MIT` | `crates.io` |
| `downcast-rs` | `1.2.1` | `MIT/Apache-2.0` | `crates.io` |
| `dpi` | `0.1.2` | `Apache-2.0 AND MIT` | `crates.io` |
| `drm` | `0.14.1` | `MIT` | `crates.io` |
| `drm-ffi` | `0.9.1` | `MIT` | `crates.io` |
| `drm-fourcc` | `2.2.0` | `MIT` | `crates.io` |
| `drm-sys` | `0.8.1` | `MIT` | `crates.io` |
| `dtor` | `0.8.1` | `Apache-2.0 OR MIT` | `crates.io` |
| `embed-manifest` | `1.5.1` | `MIT` | `crates.io` |
| `embedded-io` | `0.4.0` | `MIT OR Apache-2.0` | `crates.io` |
| `embedded-io` | `0.6.1` | `MIT OR Apache-2.0` | `crates.io` |
| `encoding_rs` | `0.8.42` | `(Apache-2.0 OR MIT) AND BSD-3-Clause` | `crates.io` |
| `equivalent` | `1.0.3` | `Apache-2.0 OR MIT` | `crates.io` |
| `errno` | `0.3.14` | `MIT OR Apache-2.0` | `crates.io` |
| `extended` | `0.1.0` | `MIT` | `crates.io` |
| `fallible-iterator` | `0.3.0` | `MIT/Apache-2.0` | `crates.io` |
| `fallible-streaming-iterator` | `0.1.9` | `MIT/Apache-2.0` | `crates.io` |
| `fastrand` | `2.5.0` | `Apache-2.0 OR MIT` | `crates.io` |
| `fdeflate` | `0.3.7` | `MIT OR Apache-2.0` | `crates.io` |
| `find-msvc-tools` | `0.1.14` | `MIT OR Apache-2.0` | `crates.io` |
| `flate2` | `1.1.10` | `MIT OR Apache-2.0` | `crates.io` |
| `fontdb` | `0.24.0` | `MIT` | `crates.io` |
| `foreign-types` | `0.5.0` | `MIT/Apache-2.0` | `crates.io` |
| `foreign-types-macros` | `0.2.4` | `MIT/Apache-2.0` | `crates.io` |
| `foreign-types-shared` | `0.3.1` | `MIT/Apache-2.0` | `crates.io` |
| `futures-core` | `0.3.34` | `MIT OR Apache-2.0` | `crates.io` |
| `futures-task` | `0.3.34` | `MIT OR Apache-2.0` | `crates.io` |
| `futures-util` | `0.3.34` | `MIT OR Apache-2.0` | `crates.io` |
| `gethostname` | `1.1.0` | `Apache-2.0` | `crates.io` |
| `getrandom` | `0.3.4` | `MIT OR Apache-2.0` | `crates.io` |
| `getrandom` | `0.4.3` | `MIT OR Apache-2.0` | `crates.io` |
| `hash32` | `0.2.1` | `MIT OR Apache-2.0` | `crates.io` |
| `hashbrown` | `0.14.5` | `MIT OR Apache-2.0` | `crates.io` |
| `hashbrown` | `0.17.1` | `MIT OR Apache-2.0` | `crates.io` |
| `hashlink` | `0.9.1` | `MIT OR Apache-2.0` | `crates.io` |
| `heapless` | `0.7.17` | `MIT OR Apache-2.0` | `crates.io` |
| `hermit-abi` | `0.5.3` | `MIT OR Apache-2.0` | `crates.io` |
| `hound` | `3.5.1` | `Apache-2.0` | `crates.io` |
| `indexmap` | `2.14.2` | `Apache-2.0 OR MIT` | `crates.io` |
| `jni` | `0.22.4` | `MIT OR Apache-2.0` | `crates.io` |
| `jni-macros` | `0.22.4` | `MIT OR Apache-2.0` | `crates.io` |
| `jni-sys` | `0.3.1` | `MIT OR Apache-2.0` | `crates.io` |
| `jni-sys` | `0.4.1` | `MIT OR Apache-2.0` | `crates.io` |
| `jni-sys-macros` | `0.4.1` | `MIT OR Apache-2.0` | `crates.io` |
| `jobserver` | `0.1.35` | `MIT OR Apache-2.0` | `crates.io` |
| `js-sys` | `0.3.106` | `MIT OR Apache-2.0` | `crates.io` |
| `lazy_static` | `1.5.1` | `MIT OR Apache-2.0` | `crates.io` |
| `libc` | `0.2.190` | `MIT OR Apache-2.0` | `crates.io` |
| `libloading` | `0.8.9` | `ISC` | `crates.io` |
| `libredox` | `0.1.25` | `MIT` | `crates.io` |
| `libsqlite3-sys` | `0.30.1` | `MIT` | `crates.io` |
| `linux-raw-sys` | `0.12.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `linux-raw-sys` | `0.4.15` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `linux-raw-sys` | `0.9.4` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `lock_api` | `0.4.14` | `MIT OR Apache-2.0` | `crates.io` |
| `lofty` | `0.25.4` | `MIT OR Apache-2.0` | `crates.io` |
| `lofty_attr` | `0.13.0` | `MIT OR Apache-2.0` | `crates.io` |
| `log` | `0.4.34` | `MIT OR Apache-2.0` | `crates.io` |
| `mach2` | `0.6.0` | `BSD-2-Clause OR MIT OR Apache-2.0` | `crates.io` |
| `memchr` | `2.8.3` | `Unlicense OR MIT` | `crates.io` |
| `memmap2` | `0.9.11` | `MIT OR Apache-2.0` | `crates.io` |
| `miniz_oxide` | `0.8.9` | `MIT OR Zlib OR Apache-2.0` | `crates.io` |
| `miniz_oxide` | `0.9.1` | `MIT OR Zlib OR Apache-2.0` | `crates.io` |
| `multiversion_no_op` | `1.0.0` | `Apache-2.0 OR MIT` | `crates.io` |
| `ndk` | `0.9.0` | `MIT OR Apache-2.0` | `crates.io` |
| `ndk-context` | `0.1.1` | `MIT OR Apache-2.0` | `crates.io` |
| `ndk-sys` | `0.6.0+11769913` | `MIT OR Apache-2.0` | `crates.io` |
| `nix` | `0.31.3` | `MIT` | `crates.io` |
| `num-complex` | `0.4.6` | `MIT OR Apache-2.0` | `crates.io` |
| `num-derive` | `0.4.2` | `MIT OR Apache-2.0` | `crates.io` |
| `num-integer` | `0.1.47` | `MIT OR Apache-2.0` | `crates.io` |
| `num-traits` | `0.2.19` | `MIT OR Apache-2.0` | `crates.io` |
| `num_enum` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | `crates.io` |
| `num_enum_derive` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | `crates.io` |
| `objc-sys` | `0.3.5` | `MIT` | `crates.io` |
| `objc2` | `0.5.3` | `MIT` | `crates.io` |
| `objc2` | `0.6.5` | `MIT` | `crates.io` |
| `objc2-app-kit` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-audio-toolbox` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-avf-audio` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-cloud-kit` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-contacts` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-core-audio` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-core-audio-types` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-core-data` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-core-foundation` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-core-graphics` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-core-image` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-core-location` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-encode` | `4.1.0` | `MIT` | `crates.io` |
| `objc2-foundation` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-foundation` | `0.3.2` | `MIT` | `crates.io` |
| `objc2-io-surface` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-link-presentation` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-metal` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-quartz-core` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-quartz-core` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `objc2-symbols` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-ui-kit` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-uniform-type-identifiers` | `0.2.2` | `MIT` | `crates.io` |
| `objc2-user-notifications` | `0.2.2` | `MIT` | `crates.io` |
| `ogg_pager` | `0.7.2` | `MIT OR Apache-2.0` | `crates.io` |
| `once_cell` | `1.21.4` | `MIT OR Apache-2.0` | `crates.io` |
| `orbclient` | `0.3.55` | `MIT` | `crates.io` |
| `owned_ttf_parser` | `0.25.1` | `Apache-2.0` | `crates.io` |
| `paste` | `1.0.15` | `MIT OR Apache-2.0` | `crates.io` |
| `percent-encoding` | `2.3.2` | `MIT OR Apache-2.0` | `crates.io` |
| `pin-project` | `1.1.13` | `Apache-2.0 OR MIT` | `crates.io` |
| `pin-project-internal` | `1.1.13` | `Apache-2.0 OR MIT` | `crates.io` |
| `pin-project-lite` | `0.2.17` | `Apache-2.0 OR MIT` | `crates.io` |
| `pkg-config` | `0.3.34` | `MIT OR Apache-2.0` | `crates.io` |
| `plain` | `0.2.3` | `MIT/Apache-2.0` | `crates.io` |
| `png` | `0.18.1` | `MIT OR Apache-2.0` | `crates.io` |
| `polling` | `3.11.0` | `Apache-2.0 OR MIT` | `crates.io` |
| `postcard` | `1.1.3` | `MIT OR Apache-2.0` | `crates.io` |
| `primal-check` | `0.3.4` | `MIT OR Apache-2.0` | `crates.io` |
| `proc-macro-crate` | `3.5.0` | `MIT OR Apache-2.0` | `crates.io` |
| `proc-macro2` | `1.0.107` | `MIT OR Apache-2.0` | `crates.io` |
| `quick-xml` | `0.41.0` | `MIT` | `crates.io` |
| `quote` | `1.0.47` | `MIT OR Apache-2.0` | `crates.io` |
| `r-efi` | `5.3.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `crates.io` |
| `r-efi` | `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `crates.io` |
| `raw-window-handle` | `0.6.2` | `MIT OR Apache-2.0 OR Zlib` | `crates.io` |
| `realfft` | `3.5.0` | `MIT` | `crates.io` |
| `redox_syscall` | `0.4.1` | `MIT` | `crates.io` |
| `redox_syscall` | `0.5.18` | `MIT` | `crates.io` |
| `redox_syscall` | `0.9.4` | `MIT` | `crates.io` |
| `rtrb` | `0.4.0` | `MIT OR Apache-2.0` | `crates.io` |
| `rubato` | `5.0.1` | `MIT OR Apache-2.0` | `crates.io` |
| `rusqlite` | `0.32.1` | `MIT` | `crates.io` |
| `rustc_version` | `0.4.1` | `MIT OR Apache-2.0` | `crates.io` |
| `rustfft` | `6.4.1` | `MIT OR Apache-2.0` | `crates.io` |
| `rustix` | `0.38.44` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `rustix` | `1.1.5` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `rustversion` | `1.0.23` | `MIT OR Apache-2.0` | `crates.io` |
| `same-file` | `1.0.6` | `Unlicense/MIT` | `crates.io` |
| `scoped-tls` | `1.0.1` | `MIT/Apache-2.0` | `crates.io` |
| `scopeguard` | `1.2.0` | `MIT OR Apache-2.0` | `crates.io` |
| `sctk-adwaita` | `0.10.1` | `MIT` | `crates.io` |
| `semver` | `1.0.28` | `MIT OR Apache-2.0` | `crates.io` |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | `crates.io` |
| `serde_core` | `1.0.229` | `MIT OR Apache-2.0` | `crates.io` |
| `serde_derive` | `1.0.229` | `MIT OR Apache-2.0` | `crates.io` |
| `serde_spanned` | `1.1.2` | `MIT OR Apache-2.0` | `crates.io` |
| `shlex` | `2.0.1` | `MIT OR Apache-2.0` | `crates.io` |
| `simd-adler32` | `0.3.10` | `MIT` | `crates.io` |
| `simd_cesu8` | `1.2.0` | `Apache-2.0 OR MIT` | `crates.io` |
| `simdutf8` | `0.1.5` | `MIT OR Apache-2.0` | `crates.io` |
| `slab` | `0.4.12` | `MIT` | `crates.io` |
| `slotmap` | `1.1.1` | `Zlib` | `crates.io` |
| `smallvec` | `1.16.3` | `MIT OR Apache-2.0` | `crates.io` |
| `smithay-client-toolkit` | `0.19.2` | `MIT` | `crates.io` |
| `smol_str` | `0.2.2` | `MIT OR Apache-2.0` | `crates.io` |
| `softbuffer` | `0.4.8` | `MIT OR Apache-2.0` | `crates.io` |
| `spin` | `0.9.9` | `MIT` | `crates.io` |
| `stable_deref_trait` | `1.2.1` | `MIT OR Apache-2.0` | `crates.io` |
| `strength_reduce` | `0.2.4` | `MIT OR Apache-2.0` | `crates.io` |
| `strict-num` | `0.1.1` | `MIT` | `crates.io` |
| `symphonia` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-bundle-flac` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-bundle-mp3` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-codec-aac` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-codec-adpcm` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-codec-alac` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-codec-pcm` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-codec-vorbis` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-core` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-format-caf` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-format-isomp4` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-format-mkv` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-format-ogg` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-format-riff` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-metadata` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `symphonia-utils-xiph` | `0.5.5` | `MPL-2.0` | `crates.io` |
| `syn` | `2.0.119` | `MIT OR Apache-2.0` | `crates.io` |
| `syn` | `3.0.9` | `MIT OR Apache-2.0` | `crates.io` |
| `tempfile` | `3.27.0` | `MIT OR Apache-2.0` | `crates.io` |
| `thiserror` | `1.0.69` | `MIT OR Apache-2.0` | `crates.io` |
| `thiserror` | `2.0.21` | `MIT OR Apache-2.0` | `crates.io` |
| `thiserror-impl` | `1.0.69` | `MIT OR Apache-2.0` | `crates.io` |
| `thiserror-impl` | `2.0.21` | `MIT OR Apache-2.0` | `crates.io` |
| `tiny-skia` | `0.11.4` | `BSD-3-Clause` | `crates.io` |
| `tiny-skia-path` | `0.11.4` | `BSD-3-Clause` | `crates.io` |
| `tiny-xlib` | `0.2.5` | `MIT OR Apache-2.0 OR Zlib` | `crates.io` |
| `tinyvec` | `1.13.3` | `Zlib OR Apache-2.0 OR MIT` | `crates.io` |
| `tokio` | `1.53.3` | `MIT` | `crates.io` |
| `toml` | `0.9.12+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `toml_datetime` | `0.7.5+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `toml_datetime` | `1.1.2+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `toml_edit` | `0.25.17+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `toml_parser` | `1.1.5+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `toml_writer` | `1.1.3+spec-1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `tracing` | `0.1.44` | `MIT` | `crates.io` |
| `tracing-core` | `0.1.36` | `MIT` | `crates.io` |
| `transpose` | `0.2.3` | `MIT OR Apache-2.0` | `crates.io` |
| `ttf-parser` | `0.25.1` | `MIT OR Apache-2.0` | `crates.io` |
| `typed-path` | `0.12.3` | `MIT OR Apache-2.0` | `crates.io` |
| `unicode-ident` | `1.0.27` | `(MIT OR Apache-2.0) AND Unicode-3.0` | `crates.io` |
| `unicode-segmentation` | `1.13.3` | `MIT OR Apache-2.0` | `crates.io` |
| `vcpkg` | `0.2.15` | `MIT/Apache-2.0` | `crates.io` |
| `version_check` | `0.9.5` | `MIT/Apache-2.0` | `crates.io` |
| `visibility` | `0.1.1` | `Zlib OR MIT OR Apache-2.0` | `crates.io` |
| `walkdir` | `2.5.0` | `Unlicense/MIT` | `crates.io` |
| `wasapi` | `0.26.0` | `MIT` | `crates.io` |
| `wasip2` | `1.0.4+wasi-0.2.12` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `wasm-bindgen` | `0.2.129` | `MIT OR Apache-2.0` | `crates.io` |
| `wasm-bindgen-futures` | `0.4.79` | `MIT OR Apache-2.0` | `crates.io` |
| `wasm-bindgen-macro` | `0.2.129` | `MIT OR Apache-2.0` | `crates.io` |
| `wasm-bindgen-macro-support` | `0.2.129` | `MIT OR Apache-2.0` | `crates.io` |
| `wasm-bindgen-shared` | `0.2.129` | `MIT OR Apache-2.0` | `crates.io` |
| `wayland-backend` | `0.3.17` | `MIT` | `crates.io` |
| `wayland-client` | `0.31.15` | `MIT` | `crates.io` |
| `wayland-csd-frame` | `0.3.0` | `MIT` | `crates.io` |
| `wayland-cursor` | `0.31.14` | `MIT` | `crates.io` |
| `wayland-protocols` | `0.32.13` | `MIT` | `crates.io` |
| `wayland-protocols-plasma` | `0.3.12` | `MIT` | `crates.io` |
| `wayland-protocols-wlr` | `0.3.12` | `MIT` | `crates.io` |
| `wayland-scanner` | `0.31.11` | `MIT` | `crates.io` |
| `wayland-sys` | `0.31.11` | `MIT` | `crates.io` |
| `web-sys` | `0.3.106` | `MIT OR Apache-2.0` | `crates.io` |
| `web-time` | `1.1.0` | `MIT OR Apache-2.0` | `crates.io` |
| `winapi-util` | `0.1.11` | `Unlicense OR MIT` | `crates.io` |
| `windowfunctions` | `0.1.1` | `MIT` | `crates.io` |
| `windows` | `0.62.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-collections` | `0.3.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-core` | `0.62.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-future` | `0.3.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-implement` | `0.60.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-interface` | `0.59.3` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-link` | `0.2.1` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-numerics` | `0.3.1` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-result` | `0.4.1` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-strings` | `0.5.1` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-sys` | `0.52.0` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-sys` | `0.59.0` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-sys` | `0.61.2` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-targets` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows-threading` | `0.2.1` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_aarch64_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_aarch64_msvc` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_i686_gnu` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_i686_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_i686_msvc` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_x86_64_gnu` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_x86_64_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `windows_x86_64_msvc` | `0.52.6` | `MIT OR Apache-2.0` | `crates.io` |
| `winit` | `0.30.13` | `Apache-2.0` | `crates.io` |
| `winnow` | `0.7.15` | `MIT` | `crates.io` |
| `winnow` | `1.0.4` | `MIT` | `crates.io` |
| `wit-bindgen` | `0.57.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `crates.io` |
| `x11-dl` | `2.21.0` | `MIT` | `crates.io` |
| `x11rb` | `0.13.2` | `MIT OR Apache-2.0` | `crates.io` |
| `x11rb-protocol` | `0.13.2` | `MIT OR Apache-2.0` | `crates.io` |
| `xcursor` | `0.3.11` | `MIT` | `crates.io` |
| `xkbcommon-dl` | `0.4.2` | `MIT` | `crates.io` |
| `xkeysym` | `0.2.1` | `MIT OR Apache-2.0 OR Zlib` | `crates.io` |
| `zerocopy` | `0.8.63` | `BSD-2-Clause OR Apache-2.0 OR MIT` | `crates.io` |
| `zerocopy-derive` | `0.8.63` | `BSD-2-Clause OR Apache-2.0 OR MIT` | `crates.io` |
| `zip` | `9.0.3` | `MIT` | `crates.io` |
| `zlib-rs` | `0.6.8` | `Zlib` | `crates.io` |
| `zopfli` | `0.8.4` | `Apache-2.0` | `crates.io` |
<!-- END CARGO LICENSE INVENTORY -->
