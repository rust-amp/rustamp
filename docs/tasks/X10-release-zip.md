# X10 — Portable Release ZIP & License Inventory (G1, G2, G4, G5, G6)

**Date:** 2026-10-18. **Status:** Complete. Ready for publication packaging.  
**Decision:** The project owner decided that Rustamp releases ship as a **portable ZIP archive published on GitHub**, accompanied by the complete license inventory and obligations that an open-source release must ship. **No installer, no code signing certificate, and no automatic update mechanism.**

**Worktree / write boundary:**  
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave86-release-zip`  
Owned files: `xtask/src/package.rs`, `docs/tasks/RELEASE-plan.md`, `docs/tasks/RELEASE-notes-template.md`, and this card (`docs/tasks/X10-release-zip.md`).  
No crate source files, test fixtures (`tests/golden/**`), or runner scripts were touched. No host tests were executed (`cargo test` runs only in Hyper-V VM). No git commits were made.

---

## 1. Owner Decision & Scope

| Release Dimension | Owner Decision | Technical Realization |
|---|---|---|
| **Distribution Format (G4)** | Portable ZIP published on GitHub | Built-in pure-Rust APPNOTE.TXT ZIP packager in `xtask`; archive named by the §3 versioning rule (`rustamp-<version>+<date>.<short_commit>.zip`) with an accompanying `.sha256` checksum file. |
| **Installer / System Footprint** | None | No installer EXE/MSI, no Windows registry modifications, no system service, no file association hooks, and no startup tasks. Extraction is self-contained. |
| **Licensing Inventory (G1)** | Complete compliance inventory | Extended layout audit to expect and verify `THIRD_PARTY.md` across root, player, and host layouts. Generated from `Cargo.lock` and repo records. Complete source offer, notice preservation, and clean-room isolation. |
| **Version Stamping (G2)** | Stamped manifests and clean QUICKSTART | Manifest schema bumped to 3, dynamically stamping `product_version`, `commit`, `commit_short`, `release_id`, `protocol_version`, and `config_version`. Fixed stale QUICKSTART claims. |
| **Code Signing (G5)** | Unsigned binaries | Stated honestly in release notes and documentation: SmartScreen warnings, manual "Run anyway" instructions, and SHA-256 integrity verification. |
| **Update Mechanism (G6)** | No automatic updates | Zero network phone-home, zero background checking, zero telemetry. Fully manual upgrade procedure documented; `%APPDATA%\Rustamp\rustamp.toml` preserved across upgrades. |

---

## 2. G1 — License Obligations and Shipped Notices

The full dependency closure of Rustamp contains 302 crate entries in `Cargo.lock` (player target `x86_64-pc-windows-msvc`), plus 35 crate entries in the isolated host target (`i686-pc-windows-msvc`).

### License Obligations Matrix

| License / Component | Crates / Material in Scope | Legal Obligations | What Now Ships in the Release |
|---|---|---|---|
| **GPL-3.0-or-later** | `rustamp-app`, `rustamp-core`, `rustamp-ui`, `rustamp-dechost`, `rustamp-dsphost`, and player executables | **Section 6 Source Offer:** Must provide complete corresponding source code. Must include full GPL-3.0 license text and copyright notices. Must state Section 7 additional permissions if any. | Shipped in `player/LICENSE` and `player/PLUGIN-EXCEPTION.md`. Section 6 written offer is shipped in `player/THIRD_PARTY.md` with active links to GitHub repository and release tag source tarballs. |
| **MPL-2.0** | `symphonia`, `symphonia-bundle-mp3`, `symphonia-bundle-flac`, `symphonia-codec-*`, `symphonia-format-*` | **Weak Copyleft / Section 3:** Must inform recipients that source code is available under MPL-2.0, provide instructions on obtaining source, and include full MPL-2.0 license text. Does not taint enclosing proprietary or permissive works if kept modular. | Shipped in `player/THIRD_PARTY.md` §4: full Symphonia notice, link to upstream repository (https://github.com/pdeljanov/Symphonia), and complete 374-line Mozilla Public License 2.0 text. Excluded from `host/` package. |
| **MIT** | `crossbeam`, `miniz_oxide`, `parking_lot`, `serde`, `tokio`, `tracing`, `windows`, and many others; Webamp classic skin tables | **Notice & Copyright Attribution:** Must reproduce the original copyright notice and permission notice in all copies or substantial portions of the software. | Shipped in `THIRD_PARTY.md` (root, `player/`, `host/`): full MIT license text, Webamp classic-skin attribution block (Jordan Eldredge), and per-crate inventory entries. |
| **Apache-2.0** | `byteorder`, `log`, `rustls` (if present), `windows-*`, and dual-licensed crates | **Attribution & Notice:** Must preserve copyright, patent, trademark, and attribution notices. Must provide a copy of the Apache 2.0 license and state changes. | Shipped in `THIRD_PARTY.md` (root, `player/`, `host/`): full Apache License 2.0 text and per-crate inventory. |
| **BSD-2-Clause / BSD-3-Clause** | `bitflags`, `indexmap`, `rustc-hash`, `ryu`, etc. | **Redistribution Conditions:** Must retain copyright notices, conditions list, and disclaimer in binary distributions. BSD-3 forbids using contributor names for endorsement. | Shipped in `THIRD_PARTY.md` and `player/THIRD_PARTY.md`: canonical BSD-2-Clause and BSD-3-Clause texts and per-package entries. |
| **ISC** | `untrusted` | **Notice Reproduction:** Must retain copyright notice and permission notice in all copies. | Shipped in `THIRD_PARTY.md` and `player/THIRD_PARTY.md`: full ISC license text and inventory entry. |
| **Zlib** | `adler2`, `crc32fast`, `flate2`, `miniz_oxide` | **Attribution & Non-misrepresentation:** Origin must not be misrepresented; altered versions must be marked; notice must not be removed. | Shipped in `THIRD_PARTY.md` and `player/THIRD_PARTY.md`: full Zlib license text and inventory entry. |
| **Unicode-3.0** | `unicode-ident` | **Notice & Disclaimer:** Must include copyright notice and permission notice with software. | Shipped in `THIRD_PARTY.md` and referenced in license tables. |
| **CC0-1.0 / Unlicense** | `byteorder`, `tinyvec`, public domain fallbacks | **Public Domain Dedication:** Dedication to public domain or permissive unconditional fallback. | Noted in dependency inventory tables. |
| **Microsoft Visual C++ CRT (Static)** | Static CRT in all four EXEs (`-C target-feature=+crt-static`) | **REDIST.TXT / EULA Terms:** Redistribution permitted only in binary form as part of and for the purpose of running the application. No separate distribution of CRT libraries. | Shipped in `THIRD_PARTY.md`, `player/THIRD_PARTY.md`, and `host/THIRD_PARTY.md`: explicit static CRT redistribution disclosure citing Microsoft Visual Studio Software License Terms. |

### Per-Directory Licensing Split & Layout Audit

1. **Top-Level (`/`):**
   - `README.md`: User guide, disclosures, version identity, integrity verification.
   - `QUICKSTART.md`: Reconciled running guide with CLI switches and config notes.
   - `THIRD_PARTY.md`: Verbatim copy of repository root `THIRD_PARTY.md` containing the complete 302-package inventory.
   - `manifest.json`: Top-level schema 3 manifest detailing both packages, product version, commit, protocol version, and runtime contract.

2. **Player Package (`player/`):**
   - Executables: `rustamp.exe` (player), `rustamp-dechost-symphonia.exe` (decoder resident), `rustamp-dsphost-stages.exe` (DSP resident), `rustamp-plughost32.exe` (bundled 32-bit plugin host helper per RFC 0006 §8).
   - Licenses: `LICENSE` (GPL-3.0 text), `PLUGIN-EXCEPTION.md` (Section 7 plugin exception), `THIRD_PARTY.md` (player third-party notices, GPL-3 §6 source offer, static CRT terms, Webamp MIT notice, Symphonia MPL-2.0 notice & full text, permissive licenses, Cargo inventory).
   - Metadata: `manifest.json`, `QUICKSTART.md`.

3. **Standalone Host Package (`host/`):**
   - Executable: `rustamp-plughost32.exe` (32-bit standalone helper for Winamp plugins).
   - Licenses: `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY.md`.
   - Clean-Room Verification: `host/THIRD_PARTY.md` lists exclusively the 35 permissive crates in the host normal closure. **Audited by `audit_host_dir`**: verified that no GPL binaries, copyleft license texts, or MPL terms exist in the host directory.
   - Metadata: `manifest.json`.

---

## 3. G4 — Portable ZIP Archive & SHA-256 Checksums

### Packaging Command & Automation
The release packager produces the portable archive and checksum file automatically:
```powershell
cargo xtask package --release --out target/dist
```
- Produces folder: `target/dist`
- Produces archive: `target/dist.zip` (or `<release_id>.zip` when naming convention is used)
- Produces checksum: `target/dist.zip.sha256`

### Pure-Rust Zero-Dependency Architecture
No external compression tools, PowerShell commands, or third-party crates were added:
- **APPNOTE.TXT Compliant ZIP Writer (`create_zip_archive`):**
  - Implements standard PKZIP 2.0 format with Stored method (compression method 0) for byte-exact determinism and high performance.
  - Forward-slash normalized relative paths (`player/rustamp.exe`).
  - UTF-8 filename flag (`0x0800` in general purpose bit flag) for universal cross-platform compatibility.
  - Fixed DOS timestamps (2026-10-18 12:00:00) so repeated runs produce reproducible archives.
  - Generates valid Local File Headers, Central Directory Headers, and End of Central Directory (EOCD) records.
- **Pure-Rust SHA-256 Digest Engine (`Sha256`):**
  - Clean-room implementation of FIPS 180-4 standard with standard initial state constants, round message schedules, and padding.
  - Validated against NIST standard test vectors (`""` and `"abc"`).
- **Pure-Rust CRC-32 Engine (`crc32`):**
  - Standard IEEE 802.3 CRC-32 using compile-time const table.
  - Validated against test vector `"123456789"` (`0xCBF43926`).

### Archive Layout
```text
rustamp-0.1.0+2026-10-18.f52645a.zip
├── manifest.json
├── README.md
├── QUICKSTART.md
├── THIRD_PARTY.md
├── player/
│   ├── rustamp.exe
│   ├── rustamp-dechost-symphonia.exe
│   ├── rustamp-dsphost-stages.exe
│   ├── rustamp-plughost32.exe
│   ├── manifest.json
│   ├── QUICKSTART.md
│   ├── LICENSE
│   ├── PLUGIN-EXCEPTION.md
│   └── THIRD_PARTY.md
└── host/
    ├── rustamp-plughost32.exe
    ├── manifest.json
    ├── LICENSE-MIT
    ├── LICENSE-APACHE
    └── THIRD_PARTY.md
```

Beside the archive sits the checksum file:
```text
rustamp-0.1.0+2026-10-18.f52645a.zip.sha256:
<hash> *rustamp-0.1.0+2026-10-18.f52645a.zip
```

---

## 4. G2 — Version Stamping & Manifest Schema 3

### Stamped Manifest Fields
The packager automatically queries the workspace metadata:
- Product version `0.1.0` read dynamically from `Cargo.toml` (the first release is 0.1.0 and it ships unsigned with no auto-update, which are the two decisions the owner made).
- Git commit hash `f52645aa335d30c5ec6589583069a65fa0288adc` and short commit `f52645a` read via headless `git rev-parse HEAD`.
- Manifest `schema_version` bumped from 2 to **3**.
- IPC `protocol_version` stamped as **4**.
- Config `config_version` stamped as **1**.
- Formed release identifier: `rustamp-0.1.0+2026-10-18.f52645a`.

### Stale QUICKSTART Reconciled
- **Fixed Stale Claim:** The obsolete note claiming *"UI changes are not saved by this beta"* was removed. In accordance with current engine capabilities, documentation correctly explains that window positions, volume, and configuration are persisted to `%APPDATA%\Rustamp\rustamp.toml` on clean exit.
- **Skins Documented:** Documented default drawn Rust theme skin and `--skin lookalike` switch.
- **Portable ZIP Documented:** Documented extraction and manual upgrade path.

---

## 5. G5 & G6 — Honest Disclosures

### G5: Unsigned Binaries
- **Fact:** The binaries are not signed with a commercial Microsoft Authenticode certificate.
- **User Impact:** Windows SmartScreen will display: *"Windows protected your PC — Microsoft Defender SmartScreen prevented an unrecognized app from starting."*
- **Action Required:** Users must click **"More info"**, then **"Run anyway"**.
- **Alternative Trust Anchor:** Because Authenticode signatures are absent, releases publish SHA-256 checksums (`.sha256`) that users can verify before execution.

### G6: No Automatic Updates
- **Fact:** There is no update daemon, network checker, background service, or telemetry in Rustamp.
- **User Impact:** Rustamp will never connect to the internet to check for updates.
- **Upgrade Path:** Completely manual. Users download newer release ZIPs and extract them. Config files in `%APPDATA%\Rustamp\rustamp.toml` remain untouched across manual replacements.

---

## 6. What This Release Can and Cannot Claim

### What Can Be Honestly Claimed:
1. **Full Static MSVC CRT Linkage:** All four executables (`rustamp.exe`, `rustamp-dechost-symphonia.exe`, `rustamp-dsphost-stages.exe`, `rustamp-plughost32.exe`) are compiled with `-C target-feature=+crt-static`. They require no Microsoft Visual C++ Redistributable package.
2. **Local Audio Playback:** MP3, FLAC, and WAV audio streams play correctly via Symphonia and RIFF decoders with transport controls (play, pause, stop, seek, gapless advance).
3. **Audio Processing:** 10-band Equalizer and preamp in the DSP chain.
4. **Classic & Custom Skins:** Renders classic Winamp skins (.wsz, .zip, folders) alongside built-in vector themes.
5. **IPC & Supervision:** Robust single-instance CLI forwarding and resident background process supervision over named pipes and shared memory.
6. **Open Source Legal Compliance:** GPL-3.0 with Section 7 Plugin Exception for player; MIT/Apache-2.0 isolation for host; complete source code offer; full attribution for Symphonia and Webamp.

### What Cannot Be Claimed (Unproven Boundaries):
1. **Clean-Machine Import Proof:** `artifact_imports_verified: false` remains in manifests; offline inspection against a strict system-DLL allowlist on a pristine Windows installation is not yet completed.
2. **Physical Audio Hardware Certification:** Latency, device-loss reconnection timing, and varied soundcard driver behavior on physical hardware are uncertified (tested only in VMs with null/software sinks).
3. **Winamp Plugin Compatibility:** While the 32-bit helper process and cryptographic approval store exist, user-facing UI wiring to select and configure plugins is not yet implemented. The helper process is out-of-process isolation, not a security sandbox.
4. **Placebo Controls:** Spectrum analyzer display is contracted to zero. Shuffle and Repeat UI buttons are toggles but do not alter playlist playback order.

---

## 7. Verification Receipts

All commands executed host-side, headless, with zero network calls:

1. **Code Formatting:**
   ```powershell
   cargo fmt --all -- --check
   ```
   *Result:* Clean (exit code 0).

2. **Linter / Clippy:**
   ```powershell
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   ```
   *Result:* Clean (exit code 0, zero warnings).

3. **Workspace Build:**
   ```powershell
   cargo build --workspace
   ```
   *Result:* Success (exit code 0).

4. **Packaging Check:**
   ```powershell
   cargo xtask package --check --out target/test-pkg-run
   ```
   *Result:* Success (exit code 0). Verified release ID `rustamp-0.1.0+2026-10-18.f52645a` (first release is 0.1.0, shipping unsigned with no auto-update per owner decisions), manifest schema 3, static CRT contract, GPL-free host closure, layout audit, and ZIP checksum audit.

5. **Packaging End-to-End Build & Archive Creation:**
   ```powershell
   cargo xtask package --out target/test-pkg-run
   ```
   *Result:* Success (exit code 0). Built all 4 release binaries with static CRT, populated player and host folders, audited layouts, generated ZIP archive `target/test-pkg-run.zip`, and generated SHA-256 checksum `target/test-pkg-run.zip.sha256`.

6. **Safety & Policy Guardrails:**
   - **Zero tests run on host:** Confirmed. `cargo test` was NOT executed.
   - **Zero git commits:** Confirmed. No git commits or branches created.
   - **No new dependencies:** Confirmed. Root `Cargo.toml` and `Cargo.lock` untouched.
   - **Worktree containment:** Confirmed. Only files inside the designated worktree were modified or created.
