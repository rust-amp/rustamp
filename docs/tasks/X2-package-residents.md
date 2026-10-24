# X2 — Package host residents and golden check validation

**Date:** 2026-10-17. **Status:** packaging of decoder and DSP residents complete;
stale-golden regression check added; headless verification complete.

**Worktree / write boundary:**
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave55-xtask-package`.
Owned files: `xtask/src/**` and this card (`docs/tasks/X2-package-residents.md`).
No access to canonical, research or reference-binary trees; write only these files
and local Cargo build outputs inside this worktree. No contract or dependency change.

---

## 1. Resident binaries and search order in the codebase

### Host resolution findings

1. **Plugin host (`rustamp-plughost32.exe`, i686):**
   - Implemented in `crates/rustamp-app/src/supervise.rs::plugin_host_candidates()`.
   - **Order:**
     1. Packaged layout (RFC 0006 §8): beside `rustamp.exe` (`dir.join("rustamp-plughost32.exe")`).
     2. Dev-tree candidates: `target/i686-pc-windows-msvc/<profile>/rustamp-plughost32.exe` and `target/<profile>/rustamp-plughost32.exe`.

2. **Decoder host resident (`rustamp-dechost-symphonia.exe`, x86_64) and DSP host resident (`rustamp-dsphost-stages.exe`, x86_64):**
   - **Finding:** There is **no search path or resolution order implemented in the player or mediahost code yet**.
   - `crates/rustamp-mediahost/src/decoder.rs` (`DecoderProcess::spawn`) and `crates/rustamp-mediahost/src/dsp.rs` (`DspProcess::spawn`) each accept an explicit `executable: &std::path::Path`. Neither crate nor `rustamp-app` provides candidate lists, heuristic locators or resolution functions for either resident.
   - **Packaging placement:** Following the instruction ("if there is no search path for them yet, say so and place them where the existing convention puts the plugin host"), both resident binaries are placed directly in the `player/` package directory **beside `rustamp.exe`**.

### Resident binary metadata

| Binary | Crate | Architecture / Target | License | Packaged location |
|---|---|---|---|---|
| `rustamp-dechost-symphonia.exe` | `rustamp-dechost` | `x86_64-pc-windows-msvc` | GPL-3.0-or-later | `player/rustamp-dechost-symphonia.exe` |
| `rustamp-dsphost-stages.exe` | `rustamp-dsphost` | `x86_64-pc-windows-msvc` | GPL-3.0-or-later | `player/rustamp-dsphost-stages.exe` |

Both residents are 64-bit GPL-3.0-or-later binaries. They are staged in the `player/` artefact alongside `rustamp.exe` and the GPL license files (`LICENSE`, `PLUGIN-EXCEPTION.md`). They are never placed in `host/`, ensuring the standalone plugin host artefact remains strictly permissive (MIT OR Apache-2.0).

---

## 2. Packaging implementation and layout

### Target and build configuration

- `xtask/src/package.rs` defines target constants for both residents:
  - `DECHOST_CRATE = "rustamp-dechost"`, `DECHOST_BIN = "rustamp-dechost-symphonia"`, `DECHOST_TARGET = "x86_64-pc-windows-msvc"`.
  - `DSPHOST_CRATE = "rustamp-dsphost"`, `DSPHOST_BIN = "rustamp-dsphost-stages"`, `DSPHOST_TARGET = "x86_64-pc-windows-msvc"`.
- When `build_player` is active (`package` default or `--player-only`), both residents are compiled using `build_artifact` with static MSVC CRT linkage (`-C target-feature=+crt-static`) into `target/package-build/`.
- Staged layout for `player/`:
  - `rustamp.exe`
  - `rustamp-dechost-symphonia.exe`
  - `rustamp-dsphost-stages.exe`
  - `rustamp-plughost32.exe` (when host is bundled)
  - `LICENSE`
  - `PLUGIN-EXCEPTION.md`
  - `manifest.json`
  - `QUICKSTART.md`

### Manifest updates

Both `player/manifest.json` (`serde_manifest_player`) and `manifest.json` (`build_top_manifest`) record:
- `decoder_host`: binary name (or relative path in top manifest), crate, target, and static CRT runtime specification.
- `dsp_host`: binary name (or relative path in top manifest), crate, target, and static CRT runtime specification.

### Runtime contract and layout audit

- `check_runtime_contract`: validates that `artifact_command` for all four binaries (`rustamp`, `rustamp-plughost32`, `rustamp-dechost-symphonia`, `rustamp-dsphost-stages`) enforces `-C target-feature=+crt-static`, `--target`, and `CARGO_TARGET_DIR`.
- `audit_package_layout`: requires `rustamp-dechost-symphonia.exe` and `rustamp-dsphost-stages.exe` to exist and be non-empty in `player/`. Missing residents or foreign files fail the audit.

---

## 3. Stale-golden check (`cargo xtask golden-check`)

### Requirement and release step use

Nothing previously guarded against committed golden images becoming stale in release/packaging steps. A new non-mutating check was added to `xtask`.

### Invocation options

1. **Subcommand:** `cargo xtask golden-check [--skin <name>]` (aliases: `check-golden`, `check-goldens`).
2. **Flag:** `cargo xtask golden --check [--skin <name>]`.

### Design and safeguards

- **Non-mutating:** `--bless` is strictly forbidden with `--check` (fails with an error). Any inherited `RUSTAMP_BLESS` environment variable is explicitly removed and forced to `"0"`.
- **Targeted:** Passes `--tests` to `cargo test` so that doc-tests across the workspace are skipped, running test binaries and unit/integration tests directly and headlessly.
- **Fail-closed:** Cleans `target/golden-out/` before execution. If libtest exits with failure OR any mismatch artefact (`.actual.png` or `.diff.png`) is found in `target/golden-out/`, the command reports the failure, prints the mismatch files, and terminates with exit code 1.
- **Usable from CI / release step:** Fully automated and headless; no windows, no audio device required.

---

## 4. Verification and boundary receipt

All commands were executed in worktree `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave55-xtask-package`.

- `cargo fmt --all -- --check`: passed (exit code 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed with 0 warnings (exit code 0).
- `cargo build --workspace`: passed (exit code 0).
- `cargo xtask package --check`: passed (exit code 0). Verified closure, 4-target static CRT runtime contracts, and dry-run packaging layout.
- `cargo xtask golden-check`: executed headlessly in check mode; detected 14 mismatches in `golden_lookalike` (resulting from the default skin switch in Wave 10), listed 28 mismatch artefacts in `target/golden-out`, and exited with code 1 as required.
- `cargo xtask golden --check`: identical behavior and exit code 1.
- 16 headless filesystem/manifest unit tests in `xtask` passed.

### Restraint confirmations

- No `cargo test` was executed on the host.
- No window or GUI was opened.
- No git commit was created.
- No files outside the worktree were read or written.
- `git status --short` shows only `xtask/src/main.rs`, `xtask/src/package.rs`, and this card.
