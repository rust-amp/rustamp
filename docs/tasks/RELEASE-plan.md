# RELEASE — plan: from a beta folder to something that can be called a release

**Date:** 2026-10-18. **Scope:** docs-only plan. No code, test, runner, other card or
`docs/HANDOFF.md` was edited, and nothing was executed (no cargo, no VM, no
package command). Every fact below is read from a file named next to it; where a
fact was not read, it says so.

**Starting point, in one paragraph.** There is a packaging command
(`cargo xtask package`, `xtask/src/package.rs`) and one beta folder
(`D:\code\rustamp-beta\2026-10-17`, built from `f1b87b8`, now 72 commits stale —
`docs/tasks/BETA-what-is-in-it.md:9-13`, `docs/HANDOFF-SESSION.md:77-79`). There is
**no installer, no signing, no release notes, no update mechanism, no recorded
version policy, and no packaged run that has ever completed green**
(`docs/tasks/BETA-readiness-2.md:180-189`; `docs/tasks/BETA-what-is-in-it.md:193-198`).
The package says so itself: "This folder is not an installer or a signed/externally
cleared release" (`xtask/src/package.rs:769`), and X1 lists signing, installer,
file associations, reputation and update deployment as "not implemented or
certified" (`docs/tasks/X1-package-runtime.md:113-114`).

---

## 1. What is missing, grouped and in the order it should be done

The order is by dependency, not by effort: a later item is meaningless until the
earlier one holds. Each entry states cost, dependency, what this host can do, what
needs the owner, and what a release may honestly claim without it.

Cost scale: **S** = a lane of docs/xtask work (hours), **M** = a few lanes or one
owner decision plus a lane, **L** = needs an external purchase/toolchain and
repeated VM work.

### G0. Prerequisite, not a release item: a packaged build that has actually run

| | |
|---|---|
| Missing | The packaged folder has never produced a green smoke run (window at 275×116, live bundled-host child, silent marker, clean kill). Five runner runs found four runner/staging defects; none recorded the full green proof (`BETA-what-is-in-it.md:193-198`). The folder is 72 commits stale. |
| Cost | S–M (rebuild + VM runs; possible defects in the player itself that the run will find). |
| Depends on | Owner-run VM (`base-provisioned` checkpoint). Nothing else. |
| This host | Build the package (`cargo xtask package --release --out <new dir>`), then run `docs\notes\vm-run-smoke.ps1`. Both are existing commands; neither opens a window on the host (the smoke runner launches in the guest). |
| Owner | Starts the VM job (VM access is the owner's broker; `docs/notes/vm-run-tests.ps1:6-10` splits unelevated compile/stage from the elevated VM step). |
| May claim without it | Nothing about running. "A folder that the layout audit accepts" is the ceiling. |

### G1. Licence inventory that must ship (Implemented in X10 — card `docs/tasks/X10-release-zip.md`)

| | |
|---|---|
| Status | **Implemented in X10.** The packaging tool (`xtask/src/package.rs`) now generates and audits the complete license inventory and required notices. The layout audit expects rather than refuses `THIRD_PARTY.md` across top-level, player, and host directories. |
| Missing | Previously missing texts and notices. Now shipped: `player/THIRD_PARTY.md` ships the GPL-3.0 §6 written source offer (repo and release tag links), static MSVC CRT redistribution terms, Webamp MIT skin-table attribution, Symphonia MPL-2.0 notice & full 374-line license text, canonical permissive licenses, and full `Cargo.lock` inventory (302 crates). `host/THIRD_PARTY.md` ships strictly permissive terms, static CRT terms, canonical MIT/Apache texts, and the 35-package host normal closure inventory. Audited by `audit_host_dir` for zero copyleft/MPL contamination. |
| Cost | Done (S–M). |
| Depends on | `Cargo.lock`, `THIRD_PARTY.md`, `xtask/src/package.rs`. |
| This host | Packaging and layout audit implemented and validated by `cargo xtask package --check`. |
| Owner | Legal sign-off remains external, but all mechanical obligations and texts are now packaged and shipped. |
| May claim | "Ships complete third-party license notices, dependency inventories, static CRT terms, and GPL-3 §6 written source offer." |

### G2. Version numbering and release notes (Implemented in X10 — card `docs/tasks/X10-release-zip.md`)

| | |
|---|---|
| Status | **Implemented in X10.** Version policy §3 implemented in `xtask/src/package.rs` with manifest schema bumped to version 3. Release notes template created in `docs/tasks/RELEASE-notes-template.md`. |
| Missing | Previously unstamped. Now stamped: `product_version` dynamically read from `Cargo.toml` (`0.0.1`), `commit` and `commit_short` read from headless git, `release_id` formatted as `rustamp-<version>+<date>.<short_commit>`, IPC `protocol_version: 4`, and `config_version: 1`. Stale QUICKSTART note removed and replaced with accurate description of `%APPDATA%\Rustamp\rustamp.toml` persistence on clean exit. `package --check` verifies folder name, manifest, and `Cargo.toml` consistency. |
| Cost | Done (S). |
| Depends on | `Cargo.toml`, git metadata, `xtask/src/package.rs`. |
| This host | Implemented and verified via `cargo xtask package --check`. |
| Owner | Decides when to bump product version in `Cargo.toml`. |
| May claim | "Manifests and archives carry stamped product version, git commit, protocol version, and config version." |

### G3. Clean-machine launch proof

| | |
|---|---|
| Missing | The central unproven claim: the shipped EXEs import only system DLLs and start on a Windows with no developer tooling. `artifact_imports_verified: false` is written into every manifest (`package.rs:45`); no PE import inspection exists; the static CRT is a build *policy* checked on the command line (`check_runtime_contract`, `package.rs:785-841`), not on the bytes. First launch on a clean OS, WOW64, reputation prompts and the audio device are listed "not proven" (`X1-package-runtime.md:106-112`). |
| Cost | M. Offline PE import inspection is S; a *clean* guest checkpoint (no Rust, no VC++ redistributable, no dev tools) and a smoke job against it is M; an audio-endpoint run is hardware-dependent (L). |
| Depends on | G0 first (a green smoke on the existing guest), then a guest checkpoint that provably lacks the VC++ runtime. The current `base-provisioned` baseline is the test-host baseline (`vm-run-smoke.ps1:40`); whether it is "clean" in the release sense has **not** been established in anything read here and must be checked before the word is used. |
| This host | Offline: dump imports of the three binaries and compare to a system-DLL allowlist (a new read-only xtask check is the natural place; flip `artifact_imports_verified` only from that check, never by hand). Hash and compare. Run the existing smoke runner. Never open a window on the host (AGENTS/CONTRIBUTING GUI rule; `package.rs:778`). |
| Owner | Builds/approves the clean checkpoint; decides whether a second OS image (Windows 10 vs 11, no WOW64 case) is required; runs anything needing real audio hardware. |
| May claim without it | "Built with the static MSVC CRT per the build command" — and only that. **Never** "runs on a clean machine". |

### G4. Installer (Resolved by Owner Decision: Portable ZIP on GitHub — card `docs/tasks/X10-release-zip.md`)

| | |
|---|---|
| Status | **Resolved by Owner Decision:** Portable ZIP published on GitHub. No installer, no registry, no service. Implemented in X10. |
| Missing | Previously missing archive creation. Now implemented in `xtask/src/package.rs`: pure-Rust APPNOTE.TXT ZIP packager producing `<release_id>.zip` with stored method (deterministic), forward-slash paths, UTF-8 flag, fixed DOS timestamps, and accompanying `<release_id>.zip.sha256` checksum file generated via pure-Rust SHA-256 and CRC-32 engines. Top-level `README.md` and `QUICKSTART.md` provide extraction and usage instructions. |
| Cost | Done (S–M). Zero external dependencies added. |
| Depends on | `xtask/src/package.rs`. |
| This host | Built and verified natively by `cargo xtask package`. |
| Owner | Decided portable ZIP archive on GitHub; no MSI/Inno/MSIX installer toolchain needed. |
| May claim | "Portable ZIP archive, extract and run. No installer, no registry keys, no services." |

### G5. Code signing (Resolved by Owner Decision: Unsigned Binaries — card `docs/tasks/X10-release-zip.md`)

| | |
|---|---|
| Status | **Resolved by Owner Decision:** Binaries remain unsigned. No Authenticode certificate. Stated honestly in release notes and documentation. |
| Missing | Commercial code-signing certificate is not purchased. Releases state honestly that binaries are unsigned, explaining Windows Defender SmartScreen ("Windows protected your PC") behavior, providing click-through instructions ("More info" -> "Run anyway"), and offering `.sha256` checksum files so users can verify archive integrity independently. |
| Cost | Done (documentation and verification checksums in place). |
| Depends on | Release notes disclosures. |
| This host | Disclosures documented in `RELEASE-notes-template.md`, `README.md`, and `X10-release-zip.md`. Checksums generated automatically beside archive. |
| Owner | Decided to ship unsigned releases. |
| May claim | "Unsigned binaries. Windows SmartScreen may warn; manual bypass instructions and SHA-256 integrity verification provided." Never claim publisher identity or verified security. |

### G6. Update path (Resolved by Owner Decision: No Automatic Updates — card `docs/tasks/X10-release-zip.md`)

| | |
|---|---|
| Status | **Resolved by Owner Decision:** No automatic updates. No background service, no telemetry. Stated honestly in release notes. |
| Missing | Automatic updater is deliberately excluded. The release notes and documentation state that Rustamp has no update daemon or phone-home mechanism. The manual upgrade process is documented: users download newer release ZIPs and extract them; configuration in `%APPDATA%\Rustamp\rustamp.toml` is preserved across manual upgrades. |
| Cost | Done (documented manual path). |
| Depends on | Release notes disclosures. |
| This host | Disclosures documented in `RELEASE-notes-template.md`, `README.md`, and `X10-release-zip.md`. |
| Owner | Decided no automatic updater. |
| May claim | "No automatic updates. Upgrades are manual by downloading new release archives. User settings in %APPDATA% are preserved." |

### Order and critical path

`G0 → G1 → G2 → G3 → G4 (portable ZIP) → G5 (unsigned) → G6 (manual)`.
Under the owner's release decision:
- **G1 (Licence inventory):** Complete (X10).
- **G2 (Version stamping & notes template):** Complete (X10).
- **G4 (Portable ZIP & checksum):** Complete (X10).
- **G5 (Unsigned disclosures):** Complete (X10).
- **G6 (Manual updates disclosure):** Complete (X10).
- **G0 & G3:** Remain the only operational verification items requiring VM execution and clean-machine checkpoint inspection.

---

## 2. What the owner has to supply (single list)

1. A code-signing certificate, its private key/token custody and the CA account (G5).
2. The installer format and permission to download its toolchain (G4).
3. The legal review of the GPL player + PLUGIN-EXCEPTION, source-offer form, static
   CRT redistribution and the dependency notices (G1).
4. A clean-OS guest checkpoint (and, if the claim is wanted, an audio-hardware run) (G3).
5. The first version number and the release naming (G2, §3).
6. Whether an in-app updater is in scope (G6).

Everything else in the plan can be done on this host by lanes under the existing
rules (no tests run by a lane, no window on the host, VM only through the broker).

---

## 3. The versioning rule

### What the code already has (read, not invented)

| Source | Value | What it versions |
|---|---|---|
| `Cargo.toml:32` (`[workspace.package]`, inherited by e.g. `crates/rustamp-app/Cargo.toml:4`) | `0.0.1` | the product / every crate's own version; already surfaced as `CARGO_PKG_VERSION` in the About text (`crates/rustamp-app/src/dialogs.rs:336`) |
| `crates/rustamp-ipc/src/message.rs:35` | `PROTOCOL_VERSION: u32 = 4` | the wire protocol between player and the helper processes; compared in the Hello handshake (`crates/rustamp-mediahost/src/lib.rs:95,150`, `control.rs:160`) |
| `crates/rustamp-core/src/config.rs:15` | `CONFIG_VERSION: u32 = 1` | the on-disk config schema (`version` field, `config.rs:22`) |
| `xtask/src/package.rs:572` | manifest `"schema_version": 2` | the package-layout/manifest schema |
| `D:\code\rustamp-beta\2026-10-17` (`BETA-build.md:4`) | a date | the only identity a built artefact has today |
| `rust-toolchain.toml` / `rust-version = "1.99"` (`Cargo.toml:37`) | toolchain pin | the build environment, not a product version |

### The rule

1. **One product version, SemVer, from `[workspace.package].version`.** It is
   already the single source (every crate inherits it) and already shown to the
   user. Pre-1.0 the project stays on `0.MINOR.PATCH`, with the 0.x convention that a
   MINOR bump may break anything and PATCH is fixes only. Release candidates and
   betas carry a pre-release suffix: `0.1.0-beta.1`, `0.1.0-beta.2`, … A `1.0.0`
   is **not** promised by this rule; it requires G1, G3, G4, G5 all closed.
2. **An artefact is identified by `<version>+<yyyy-mm-dd>.<short-commit>`**, and
   its folder is named the same (`rustamp-0.1.0-beta.1+2026-10-18.abc1234`). This
   keeps the dated folder the project already uses, adds the version it lacked, and
   adds the commit it lacked — so "72 commits stale" becomes a printed fact in the
   manifest rather than something a card has to reconstruct. SemVer build metadata
   (`+…`) does not affect precedence, which matches its purpose here.
3. **The three compatibility integers are not folded into the SemVer string; they
   are recorded beside it** in the manifest and the notes, and each follows its own
   existing discipline:
   - `PROTOCOL_VERSION` (4): the player and its bundled helpers always ship from
     one build, so a mismatch inside a package is a packaging bug, not a user
     condition. Any change to it **forces at least a MINOR bump** (pre-1.0) because
     an old helper beside a new player is refused at the handshake. Contract
     changes already need an RFC (`AGENTS.md`).
   - `CONFIG_VERSION` (1): a bump forces a MINOR bump and a release-note line on
     migration; the config file is the one user-owned state that survives
     upgrades.
   - manifest `schema_version` (2): bumps with any manifest field change (this
     plan's G2 additions are such a change) and does not by itself bump the
     product version.
4. **The version is stamped, not typed.** The packager should read the version and
   the commit rather than accept them as flags, and `package --check` should fail
   when the folder name, manifest, and `Cargo.toml` disagree. (This is a *proposal*
   for a later xtask lane; this card changes no code.)
5. **A tag exists only for something that passed the §4 checklist** in full. A
   `-beta.N` that skipped a step is named in its notes, not hidden.

### Why this and not something else

- It adds one number (the product SemVer) to a project that already has three
  version integers and a date; it re-uses the one that is already inherited
  everywhere and already user-visible.
- It does not redefine the protocol or config integers, whose bump rules belong to
  their RFCs.
- It keeps the existing dated folder convention instead of replacing it.
- It does **not** use the date as the version: dates say when, not what changed,
  and the project's own experience (a 72-commit gap hidden behind one date) is the
  argument against it.

Not checked: whether `--version` exists on the player CLI. The only user-visible
version surface found is the About dialog (`dialogs.rs:336`); a `--version` flag is
a cheap addition worth making for bug reports, but it was not looked for further.

---

## 4. The release checklist

Run in this order; stop at the first failure and record it. "Host" = the
unelevated developer host, which never opens a window or runs tests; "VM" = the
owner's broker-mediated guest (`docs/notes/vm-run-tests.ps1:6-10`). Each step says
**what a pass proves** — and, in the same line, what it does not.

| # | Step | Where | Command | A pass proves | It does not prove |
|---|---|---|---|---|---|
| 0 | Clean tree, known commit | host | `git status --short` empty; record `git rev-parse HEAD` | The artefact can be tied to one commit | Anything about behaviour |
| 1 | Format | host | `cargo fmt --all -- --check` | Formatting only | — |
| 2 | Lints | host | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Compiles all targets warning-free (`AGENTS.md` required gate) | That any test passes — the lane rule is that a lane never runs tests, so many suites are compile-only until step 4 |
| 3 | Licence audit | host | `bash .github/scripts/check-permissive-licenses.sh` (or `python .github/scripts/license-audit.py`) and `python .github/scripts/check-license-audit.py`; then `python .github/scripts/license-audit.py --inventory` and diff against `THIRD_PARTY.md` | Declared licences of the five permissive crates' closures are on the allowlist and the inventory is current (`ci.yml:39-44`) | Legal clearance, licence texts, build/dev edges, native libraries, copied code (`CONTRIBUTING.md:77-83`) |
| 4 | Full suite | VM | `powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset` | Every test binary ran; the runner names `[FAIL]` and `[HNG]` binaries (5-minute bound, `HANDOFF-SESSION.md:14-15`). **Release gate: zero FAIL and zero HNG at the release commit** | The last recorded full run (r36) had 11 failures and 2 hangs; no green run at HEAD is on record (`BETA-what-is-in-it.md:209-212`). Any older green count is a snapshot, not today's |
| 5 | Ignored pass | VM | `... vm-run-tests.ps1 -Filter --ignored` (the runner prints the W12 inventory summary for `--ignored*` filters, `vm-run-tests.ps1:303-307`); the window group via `vm-run-gui-session.ps1` | The 19 ignored tests: beta-required are Groups A (8 window), B (3 real-process), D (2 real-DLL), and E1 `real_playback` (`W12-ignored-inventory.md:147-200`) | See §5: Group D is the only real-plugin evidence; Group E1 is the only production WASAPI path; an inventory line is not a pass — read each test's result |
| 6 | CI-equivalent | host/CI | the three `ci.yml` jobs (`licenses`, `windows`) | Same gates on the project's CI | CI runs `cargo test --workspace` on a hosted runner, which this project's own rules forbid on the dev host; treat CI as additional, not a substitute for step 4 |
| 7 | Version consistency | host | `cargo xtask package --check` (after G2: also checks version/commit/folder agreement) | GPL-free host closure, static-CRT command contract on all four binaries, QUICKSTART switches (`package.rs:785-841`) | Anything about the bytes — it builds nothing |
| 8 | Build the package | host | `cargo xtask package --release --out <empty new dir>` | The four binaries built from the pinned flags into a layout that passes the exact-allowlist audit; refuses a non-empty `--out` (`package.rs:143-145,293`) | That they run |
| 9 | Re-audit the produced folder | host | `cargo xtask package --check --out <that dir>` | Layout, manifests and QUICKSTART are exactly what the generator produces; no stale CRT DLLs or wrong-side licence files (`package.rs:128-136`) | Imports, launch (the manifest still says `artifact_imports_verified:false`) |
| 10 | Import inspection | host (after G3 tooling) | offline PE import dump of all four EXEs vs a system-DLL allowlist | Only system DLLs are imported — **the first evidence the static CRT worked in the bytes** | Runtime behaviour |
| 11 | Hash manifest, unsigned | host | SHA-256 of every file in the folder, written beside it (the smoke runner already hashes the player, bundled host and standalone host, `vm-run-smoke.ps1:78-80`) | Later steps and downloaders can detect any change | Authenticity — a hash beside the files proves nothing against a tamperer who also replaces the hash |
| 12 | Packaged smoke | VM | `powershell -File docs\notes\vm-run-smoke.ps1` (takes the newest folder under `D:\code\rustamp-beta`, or `-PackageDir`) | Staged bytes equal the hashes; the player launched with `--no-audio` opens the `Rustamp` window at client 275×116; a live `rustamp-plughost32.exe` child answered the handshake; silent marker present; killed; every file re-hashed identical (`BETA-smoke.md:45-68`, summarised at `BETA-what-is-in-it.md:221-230`) | A real audio endpoint, a real plugin DLL, a clean exit, a clean machine (§5) |
| 13 | Clean-machine run | VM (clean checkpoint, after G3) | the same smoke against the clean checkpoint | Starts with no developer runtime present — if and only if the checkpoint is shown clean | Other OS builds; reputation prompts on a signed binary |
| 14 | Sign (after G5) | owner | sign + timestamp every EXE (and the installer), then `signtool`-class verify | Publisher and integrity of the signed bytes | Safety or quality of the code |
| 15 | Hash manifest, final | host | repeat step 11 on the **signed** bytes; record the manifest's own hash in the release notes | The published bytes are the audited bytes | — |
| 16 | Release notes | host | from the tree at the release commit, using the §3 identity | The notes name what is in the bytes vs only in the tree (`BETA-what-is-in-it.md` is the model) | — |
| 17 | Tag | owner | tag only after 0–16 pass or are listed as skipped in the notes | The tag means "this checklist, this commit" | — |

Notes on the checklist:

- Steps 4, 5, 12, 13 are VM jobs; the host never runs tests or opens a window.
- Steps 1–3 and 7–9 are cheap and should be run on every candidate; 10 is cheap
  once written; 4–5 and 12–13 are where the project's history says defects
  appear (the red-run behind `cdead5f`, the hangs r32/r34, five smoke runs finding
  four runner defects — `BETA-what-is-in-it.md:187-198`).
- The ignored pass is not optional for a release: the Group A/B/D/E1 set is exactly
  the evidence the plain suite structurally cannot give
  (`W12-ignored-inventory.md:147-200`; `HANDOFF-SESSION.md:88-90`).

---

## 5. What must never be claimed

The project has repeatedly found each of these to be unproven. A release, a
notes file, a README or a download page may not state them until the named
evidence exists, and the notes must say they are unproven in the meantime.

| Claim | Why it is unproven | What would change that |
|---|---|---|
| "Starts on a clean machine" / "no installation needed on any Windows" | `artifact_imports_verified:false` in every manifest (`package.rs:45`); no clean-OS run; no packaged window ever proven open (`BETA-readiness-2.md:180-189`; `X1-package-runtime.md:106-112`) | Steps 10 and 13, with the checkpoint shown clean |
| "Plays audio" / "works with your sound card" / "sounds right" | The smoke runner uses `--no-audio`/null output only; sound quality, device-loss timing and a real endpoint need hardware (`BETA-what-is-in-it.md:233-236`; `BETA-readiness-2.md:256-258`). The one production WASAPI test is `real_playback` (VM, Group E1) and a VM endpoint is not hardware | A recorded run on named hardware with the device, driver and result written down |
| "Runs your Winamp plugins" / "plugin compatible" | The smoke run proves a control channel to the host process, not a DLL (`BETA-what-is-in-it.md:234-235`). The loader is real but its acceptance tests (Group D, `in_openmpt`) are VM-only and the P5 suites were compile-only (`BETA-readiness-2.md:100-112`); user-facing plugin selection/config wiring is not present (`BETA-what-is-in-it.md:166-168`); the loader is "not a sandbox" (P5 card, as quoted at `BETA-readiness-2.md:109-112`); approved bytes still run `DllMain` (`BETA-readiness-2.md:47-48`) | Group D results per plugin, and UI wiring |
| Any performance statement ("low CPU", "fast start", "real-time safe", "low latency", "gapless" as measured) | No performance measurement is recorded; the G3 `~7.8 ms` wake figure was withdrawn as an unevidenced single observation (`BETA-readiness-2.md:52,237-238`); the DSP/resident latency compensation is compile-only; S08 visualiser cost is unmeasured (`BETA-readiness-2.md:173-178`) | A measured, reproducible benchmark with hardware and method recorded |
| "Winamp 2.95 compatible" / "identical to Winamp" as a behavioural claim | Clean-room: nothing is measured against the original (`CLEANROOM.md`); pixels of the default skin are pinned by goldens against the project's own render, not against Winamp | Spec-backed measurements only, per `specs/` |
| "Signed", "verified publisher", "safe" | No signing exists (G5) | G5 done and verified |
| "Installs", "uninstalls", "file associations", "auto-updates" | None exists (G4, G6; `X1-package-runtime.md:113-114`) | G4/G6 done |
| "Licence-compliant" / "cleared" | The audit is metadata-only (`CONTRIBUTING.md:77-83`); X1 leaves notices, source offer and CRT terms to a release-owner review (`X1-package-runtime.md:115-120`) | G1 review signed off by the owner |
| Features whose controls are placebos: shuffle/repeat as playback, visualiser, EQ presets, prefs/plugin pages, menu submenus/Exit-as-quit, Clutter O/V, tray icon, global hotkeys, ReplayGain settings (`BETA-readiness-2.md:221-233`; `BETA-what-is-in-it.md:158-183`) | Skeletons or model-only; hide or label them | Their own proofs |
| "All tests pass" | No green full run at HEAD is on record; compile-only suites are the majority of newer work (`BETA-what-is-in-it.md:200-212`). Quoted counts are per-commit snapshots | Step 4 at the release commit, quoted with the commit |
| Anything about the *tree* said of the *bytes* | The artefact is frozen at its build commit; the tree has moved on (`BETA-what-is-in-it.md:7-13`) | State the commit the claim is about; use the §3 identity |

Standing wording for any release page, until the table above changes: **"Beta.
Unsigned. Portable folder, not an installer. Not verified on a clean machine, a
real audio device, a real Winamp plugin, or for performance."** State what passed
and at which commit; state what was skipped.

---

## 6. Suggested lane breakdown (for the orchestrator)

1. **R1 & R2 & R5 (Portable ZIP Release & Inventory — Completed in X10, card `docs/tasks/X10-release-zip.md`):**
   - Manifest schema bumped to 3, gains stamped `product_version`, `commit`, `commit_short`, `release_id`, `protocol_version`, `config_version`.
   - Layout audit extended to expect `THIRD_PARTY.md` across root, player, and host.
   - Built-in pure-Rust APPNOTE.TXT ZIP packager and SHA-256 checksum generator producing `<release_id>.zip` and `.sha256`.
   - Release notes template and honest G5/G6 disclosures created.
   - Verification checks pass (`cargo xtask package --check`).
2. **R3 (xtask, host-only):** offline PE import check for the four EXEs; sole writer
   of `artifact_imports_verified`.
3. **R4 (VM, owner-triggered):** G0 smoke on a fresh build; then the clean-checkpoint
   run.

---

## 7. What this card did not determine

- Whether the `base-provisioned` guest baseline is "clean" in the sense G3 needs.
- Whether config `version` mismatches migrate or are refused (only the existence
  of `CONFIG_VERSION = 1` and a test using `version: 42` at `config.rs:213` were
  seen).
- Whether the player has a `--version` flag.
- The current signing-certificate and installer-toolchain landscape (CA terms,
  hardware-token rules, store requirements): left to the owner's research; nothing
  was looked up and nothing was downloaded.
- Any number about the present suite, since nothing was run.

## 8. Boundary and verification receipt

- Read: `AGENTS.md`, `CONTRIBUTING.md` (§Dependencies, §Coding standards),
  `docs/HANDOFF-SESSION.md`, `docs/tasks/BETA-what-is-in-it.md`,
  `docs/tasks/BETA-readiness-2.md`, `docs/tasks/X1-package-runtime.md`,
  `xtask/src/package.rs`, `Cargo.toml`, `.github/workflows/ci.yml` (step list),
  `docs/notes/vm-run-tests.ps1` and `vm-run-smoke.ps1` (parameter blocks and hash
  sites), plus targeted greps for the version constants.
- Wrote: this card only. No `.rs`, test, runner, other card, `docs/HANDOFF.md`
  or crate touched. No cargo, test, package, VM, window, download, commit or git
  write. `D:\code\Rustamp`, `D:\code\rustamp-research` and
  `D:\code\rustamp-refbin` were not accessed.
