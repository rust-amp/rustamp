# Task card W20 — Version bump to 0.1.0 and release reconciliation

**Date:** 2026-10-18. **Status:** Completed.  
**Scope:** `Cargo.toml`, `Cargo.lock`, `docs/tasks/**`, `xtask/src/**`, and this card (`docs/tasks/W20-version-010.md`).  
Strict containment: No files under `crates/**` were modified. No tests were run on the host (`cargo test` strictly prohibited), and no VM was invoked. No git writes, tags, or commits were created.

---

## 1. Context & Owner Decisions

The owner has released version **0.1.0** of the Rustamp project (clean-room Winamp 2.95 reimplementation in Rust). The release is no longer a beta: it is **0.1.0**.

The owner made two binding decisions for the first release:
1. **The first release is 0.1.0** (not a beta; pre-release suffix `-beta.1` dropped from the workspace package).
2. **It ships unsigned with no auto-update**:
   - Binaries are unsigned (G5). Windows Defender SmartScreen instructions and SHA-256 checksums are provided.
   - There is no automatic update daemon or phone-home telemetry (G6). Upgrades are manual via release ZIP archives, preserving user config in `%APPDATA%\Rustamp\rustamp.toml`.

---

## 2. New Artefact Identity

Under the packaging discipline implemented in `xtask/src/package.rs`, release artefacts are stamped dynamically following §3 of `docs/tasks/RELEASE-plan.md`:
$$\text{Release ID} = \{\text{prefix}\}\text{-}\{\text{product\_version}\}\text{+}\{\text{release\_date}\}\text{.}\{\text{commit\_short}\}$$

At HEAD (`f52645aa335d30c5ec6589583069a65fa0288adc`), date `2026-10-18`, and product version `0.1.0`:
- **New Release Identifier:** `rustamp-0.1.0+2026-10-18.f52645a`
- **Full Artefact Output Directory:** `target/package/rustamp-0.1.0+2026-10-18.f52645a`
- **Portable ZIP Archive:** `target/package/rustamp-0.1.0+2026-10-18.f52645a.zip`
- **Integrity Checksum File:** `target/package/rustamp-0.1.0+2026-10-18.f52645a.zip.sha256`
- **Sub-artefact Identities:**
  - Player only: `rustamp-player-0.1.0+2026-10-18.f52645a`
  - Plugin host only: `rustamp-plughost-0.1.0+2026-10-18.f52645a`

---

## 3. Inventory of Changes Made

### 3.1 Workspace Package Version (`Cargo.toml`)
- **File:** [Cargo.toml](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/Cargo.toml#L32)
- **Old:** `version = "0.1.0-beta.1"`
- **New:** `version = "0.1.0"`
- *Impact:* All member crates in `crates/**` inherit `[workspace.package].version` via `version.workspace = true`. None pin an independent version.

### 3.2 Workspace Dependency Lock (`Cargo.lock`)
- **File:** [Cargo.lock](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/Cargo.lock)
- **Old:** 24 workspace crate entries with `version = "0.1.0-beta.1"` (`rustamp-app`, `rustamp-audio`, `rustamp-core`, `rustamp-dechost`, `rustamp-decoders`, `rustamp-dsp`, `rustamp-dsphost`, `rustamp-gfx`, `rustamp-ipc`, `rustamp-library`, `rustamp-mediahost`, `rustamp-meta`, `rustamp-milkdrop`, `rustamp-output`, `rustamp-platform`, `rustamp-playlist`, `rustamp-plughost`, `rustamp-plugin-abi`, `rustamp-prefs`, `rustamp-proto`, `rustamp-skin`, `rustamp-testkit`, `rustamp-ui`, `rustamp-vis`, and `xtask`).
- **New:** All 24 entries updated to `version = "0.1.0"`.

### 3.3 Release Plan (`docs/tasks/RELEASE-plan.md`)
- **File:** [RELEASE-plan.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/RELEASE-plan.md)
- **Section 3.1:** Updated SemVer rule to state that the owner decided the first release is `0.1.0` (not a beta) and ships unsigned with no auto-update.
- **Section 3.2:** Updated artefact naming example from `rustamp-0.1.0-beta.1+2026-10-18.abc1234` to `rustamp-0.1.0+2026-10-18.f52645a`.
- **Section 5:** Updated standing release wording from `"Beta. Unsigned..."` to `"Version 0.1.0. Unsigned. Portable folder, not an installer. No auto-update..."` reflecting the owner's decisions.

### 3.4 Release Checklist Run (`docs/tasks/RELEASE-checklist-run.md`)
- **File:** [RELEASE-checklist-run.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/RELEASE-checklist-run.md)
- **Header:** `Product Version: 0.1.0`, `Release Identifier: rustamp-0.1.0+2026-10-18.c6a0765`. Added explicit note recording the owner's decisions (0.1.0 release, unsigned, no auto-update).
- **Steps 7, 8, 9:** Updated `xtask package --check` and build outputs to reflect version `0.1.0` and release ID `rustamp-0.1.0+2026-10-18.c6a0765`.
- **Step 16:** Updated release notes output description to `0.1.0`.
- **Step 17:** Updated target tag command to `git tag -a v0.1.0 -m "Release v0.1.0"`.

### 3.5 Packaging & License Card (`docs/tasks/X10-release-zip.md`)
- **File:** [X10-release-zip.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/X10-release-zip.md)
- **Section 3 & 4:** Updated archive names, checksum entries, manifest stamping section, and verification receipts to `rustamp-0.1.0+2026-10-18.f52645a` and product version `0.1.0`. Documented owner decisions.

### 3.6 Release Readiness & Inventory Reviews
- **Files:**
  - [BETA-what-is-in-it.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/BETA-what-is-in-it.md): Added note in scope header stating the first release is 0.1.0 (no longer beta) and ships unsigned with no auto-update per owner decisions.
  - [BETA-readiness-2.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/BETA-readiness-2.md): Added owner release decision note at the top of the Decision section.
  - [N13-rfc0010-version.md](file:///C:/Users/micro/.paseo/worktrees/3ikyavhh/paseo-wave94-version/docs/tasks/N13-rfc0010-version.md): Updated first release version note in §1 to record the subsequent owner decision bumping from `0.1.0-beta.1` to `0.1.0` (unsigned, no auto-update).

---

## 4. What Was Deliberately Left Alone & Why

1. **`crates/rustamp-app/src/app.rs:4073`**:
   - Contains `format!("{} 0.1.0-beta.1", rustamp_ui::PROGRAM_NAME)` in unit test `about_dialog_first_line_matches_workspace_version`.
   - **Reason left alone:** Explicit instruction rule: **"Do NOT touch crates/** at all."** Any modification to `crates/**` would violate the defined owned-paths boundary.
2. **`docs/tasks/M28-rfc0014-impl.md:314`**:
   - Refers to `the load-playlist contract and the 0.1.0-beta.1 version bump (21f10b9)`.
   - **Reason left alone:** This is an immutable historical record documenting git commit `21f10b9`. Historical commit logs must remain truthful to the history they describe.
3. **`docs/tasks/N13-rfc0010-version.md` (Title & §2.2)**:
   - Contains references to `0.1.0-beta.1` in the task title and historical implementation narrative.
   - **Reason left alone:** Task N13 specifically introduced the initial `0.1.0-beta.1` version in a prior wave. Preserving this context accurately records the provenance of how `0.1.0-beta.1` was created before being promoted to `0.1.0` in W20.
4. **`xtask/src/package.rs:118-126` (`ReleaseMetadata::for_testing()`)**:
   - Contains fixed dummy metadata: `product_version: "0.0.1"`, commit `d2c8e5d...`, `release_id: "rustamp-0.0.1+2026-10-18.d2c8e5d"`.
   - **Reason left alone:** Fixed mock fixture used exclusively for deterministic unit testing of layout audits and manifest emission. Modifying this fixture would break mock test assertions; instruction mandates: "Do not change any behaviour, do not touch the licence texts, and do not change the manifest schema or the protocol version."

---

## 5. Verification Gate Receipts

| Gate Command | Result | Exit Code | Notes |
|---|---|:---:|---|
| `cargo fmt --all -- --check` | Pass | `0` | All workspace files cleanly formatted |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Pass | `0` | All workspace crates compile warning-free |
| `cargo build --workspace` | Pass | `0` | Workspace builds cleanly with version 0.1.0 |
| `cargo xtask package --check` | Pass | `0` | Stamps release ID `rustamp-0.1.0+2026-10-18.f52645a`, checks static CRT and GPL-free closure |
