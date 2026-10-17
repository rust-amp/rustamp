# X1 — Self-contained package CRT policy (BETA-01)

**Date:** 2026-10-17. **Status:** packaging implementation complete; artifact
and clean-OS acceptance pending. The previously built BETA artifact was neither
read, changed, rebuilt nor executed by this lane.

**Worktree / write boundary:**
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave48-package-runtime`.
Owned files: `xtask/src/package.rs` and this card. No access to the canonical,
research or reference-binary trees; write only these files and local Cargo build
outputs inside this worktree. No contract or dependency change.

## Runtime evidence: configuration is not an import-table inspection

| Binary | Before this change | Package policy now | Evidence |
|---|---|---|---|
| `rustamp.exe` | Explicit `x86_64-pc-windows-msvc` package target, without a CRT override; the MSVC target's ordinary default is dynamic CRT linkage. No runtime payload was staged. | x64 MSVC C runtime (VCRuntime/UCRT) selected statically by `-C target-feature=+crt-static`; no VC++ redistributable or app-local CRT DLL payload. Windows system DLLs remain necessary. | `.cargo/config.toml:1-2` only defines the xtask alias; `crates/rustamp-app/Cargo.toml:11-38` declares the player and its Rust dependencies; package target constants and `artifact_command` in `xtask/src/package.rs`. |
| `rustamp-plughost32.exe` | Explicit `i686-pc-windows-msvc` package target, also without a CRT override in the old packager. The VM staging path, unlike packaging, explicitly made it static. | i686 MSVC CRT selected statically by the same package command factory. Its OS calls still require Windows system DLLs; on x64 Windows the host needs WOW64. | `crates/rustamp-plughost/Cargo.toml:13-27` names the binary, permissive dependencies and Win32 API features; `.cargo/config.toml:1-2`; `docs/notes/vm-run-tests.ps1:82-108`; package constants and `artifact_command`. |

There is **no static-CRT setting or static-CRT note in the host Cargo.toml at
this revision**. The actual opt-in is in the VM runner (lines 87-90), while
`docs/tasks/BETA-build.md:24-28` describes the intended host deployment. A
manifest's license or binary declaration alone does not configure CRT linkage.
Caller-supplied flags could have affected historical builds; the old release
EXEs' exact imports cannot be established from these files.

The host must differ in **architecture**, not necessarily in linkage policy:
classic plugin DLLs are 32-bit, while the player is x64. With both EXEs beside
one another, x86 and x64 app-local CRT DLLs can have identical basenames and
cannot be shared. Static CRT linkage avoids that collision. This change makes
**both** binaries CRT-self-contained, rather than carrying Microsoft's DLLs or
an installer/download prerequisite. It does not make Windows itself optional
or promise self-containment of arbitrary future third-party plugin DLLs.

## Implementation and generated layout

- `artifact_command` owns `CARGO_ENCODED_RUSTFLAGS` (higher precedence than
  inherited `RUSTFLAGS` / target rustflags), clears `RUSTFLAGS`, and explicitly
  selects the binary and Windows target. Explicit `--target` keeps target flags
  away from host-side procedural macros and build scripts.
- Package builds are offline/locked and use
  `target/package-build/<target>/<profile>/`, not ordinary workspace outputs or
  a caller's external `CARGO_TARGET_DIR`. Ordinary builds remain unchanged.
- A non-empty output directory is refused without deleting anything. This
  prevents reusing stale CRT DLLs or wrong-side license files; use a new `--out`.
- All three manifests record static MSVC CRT linkage, the exact flags, no
  redistributable prerequisite, an empty runtime-DLL list, continued system-DLL
  requirements and `artifact_imports_verified: false`. The player manifests
  separately record the bundled host's runtime and WOW64 prerequisite. The
  top-level schema is version 2. Package/binary licenses remain unchanged.

Default full-package output (implemented, **not generated in this lane**):

```text
manifest.json
QUICKSTART.md
player/
  rustamp.exe
  rustamp-plughost32.exe
  LICENSE
  PLUGIN-EXCEPTION.md
  manifest.json
  QUICKSTART.md
host/
  rustamp-plughost32.exe
  LICENSE-MIT
  LICENSE-APACHE
  manifest.json
```

The player retains exactly the GPL license-file set; the independent host
retains exactly the permissive set. Bundling the host does not relicense it.
`--player-only` omits the host and documents detached operation;
`--host-only` emits a helper-only guide at the package root. Debug packages
explicitly warn about the GUI safety gate and are not first-user release builds.

The generated short QUICKSTART is copied to the package root and player folder.
It says to run the player, not its helper, from PowerShell with one quoted local
media path. It explains `--skin` (optional .wsz/.zip/folder artwork, no external
skin bundled, current fallback placeholder) and `--no-audio` (silent window,
no audio-device requirement). Troubleshooting covers valid MP3/FLAC/WAV input,
Windows device/mixer/volume, removing silent-output flags, restarting after a
device change, keeping stderr startup logs, and the no-window/no-sound
`--null --seconds 5` diagnostic. It states the configuration path/override,
read-only configuration behavior, unfinished GUI/plugin paths and remaining
runtime acceptance limits. CLI/config claims are from
`crates/rustamp-app/src/cli.rs:53-63,109-139` and `config.rs:10-12,43-98`;
beta limitations are reconciled with `docs/tasks/BETA-readiness.md`.

## Headless regression checks

- Every package invocation checks the actual command factory for both targets
  and both profiles, the generated runtime declarations and QUICKSTART switches.
  `cargo xtask package --check` runs this without building packaged binaries.
- When the selected output directory already exists, `--check` also audits its
  exact file layout and generated text. Packaging performs the same audit before
  reporting success. Missing/empty files, extra DLLs/executables, wrong-side
  license files, stale metadata and missing QUICKSTARTs fail. No EXE is loaded.
- Two new filesystem/command-only unit tests cover static policy and positive /
  negative full, player-only and host-only layouts. They were type-checked by
  all-target Clippy, **not executed**; execution belongs in the Hyper-V test VM.
- The existing host closure probe is preserved and made offline/locked. It is
  still only a GPL-string guard, not a replacement for the broader all-feature,
  both-target permissive-license auditor or distribution clearance.

## Not proven / external release responsibilities

- Final PE architecture and actual imported DLLs, including any native-library
  or delay-loaded dependencies: inspect the newly packaged artifacts offline.
- First launch on a truly runtime-clean Windows OS, WOW64 availability, window
  startup, usable audio device/drivers and sound: accept in the headless test VM
  with a clean checkpoint, never by opening a player/window on the host.
- No signing, installer, file associations, SmartScreen/reputation or update
  deployment is implemented or certified by this packaging work.
- `THIRD_PARTY.md`, dependency copyright/license texts, copied-source notices,
  applicable source delivery/offer obligations, toolchain/static Microsoft CRT
  redistribution terms and any required runtime notices still need an external
  release-owner review/bundle. The two license-file sets and Cargo declarations
  are not legal clearance. No notices, license scripts or dependencies were
  changed here; no Microsoft DLL/redistributable or other download was acquired.

## Verification and boundary receipt

All commands ran from the named worktree; Cargo was offline and its target
directory was explicitly set to this worktree's `target`.

- `cargo fmt --all -- --check`: passed after fixing the initial formatting diff.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo build --workspace`: passed (ordinary dev build, not package CRT proof).
- `cargo xtask package --check`: passed; closure and runtime contract OK. No
  existing output directory was present, so artifact-layout acceptance was not
  exercised by that invocation. Cargo compiled/ran xtask, not either player or
  a test binary; the package command itself built no artifact or wrote files.
- `git diff --check`: passed.
- No `cargo test`, test binary, player, helper or reference binary was executed;
  no window was opened. No download, installation, commit or git write command.
- Initial `git status --short`: empty. Final worktree receipt: only
  ` M xtask/src/package.rs` and `?? docs/tasks/X1-package-runtime.md`.
  The forbidden trees were not read or written. No third-party source copied.
