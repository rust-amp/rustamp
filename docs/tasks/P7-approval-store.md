# P7 — Production plugin approval store and verified-bytes loading

## 1. Status and scope

- **Status**: Implemented, compile-checked only. No runtime pass claimed.
- **Boundary**: write only `crates/rustamp-plughost/**` and this card in the
  assigned worktree (`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave66-approval-store`).
  Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
  `D:\code\rustamp-refbin` was read or written.
- **Sources**: worktree copies of AGENTS.md, CONTRIBUTING.md, CLEANROOM.md,
  HANDOFF.md, PLAN.md, P3/P4/P5/P6 task cards, S09/S10 specs, FIPS 180-4 standard
  specification for SHA-256, and existing clean-room implementation/tests.
  No Winamp source, SDK material, disassembly or reference binary inspected;
  no third-party code copied, no dependency added, no lockfile touched, no license changed.
- **Licensing**: `rustamp-plughost` remains `MIT OR Apache-2.0` (RFC 0004).

## 2. What the loader needed from an approval store today

Prior to this task, task P5 implemented verified-byte snapshot loading but noted
plainly: *"The VM-only acceptance runner explicitly trusts the owner-approved
staged fixture; it is not a production approval database or a general path
allowlist... The project still needs a production approval store."*

In `crates/rustamp-plughost/src/loader.rs`:
- **What the caller must supply** (`loader.rs:626-630`):
  `pub unsafe fn load_plugin(path: &Path, approved_image: &[u8], window: &HostWindow)`
  The caller had to manually supply:
  1. `path`: candidate DLL path on disk.
  2. `approved_image`: a full in-memory byte slice `&[u8]` of the pre-approved DLL image.
  3. `window`: reference to the UI-thread Win32 host window (`Winamp v1.x`).
- **What happened when the caller supplied nothing** (`loader.rs:631-632` -> `image.rs:24-29`):
  If the caller passed an empty slice (`approved_image: &[]`), `image.require_approval(&[])`
  compared the preflighted candidate bytes `self.bytes` with `&[]`. Because any valid
  PE DLL has `self.bytes.len() > 0`, `self.bytes != &[]`, and it immediately returned
  `Err(PluginLoadError::NotApproved)`.
- **What the loader does with an unapproved image** (`loader.rs:632`):
  `image.require_approval(approved_image)?` returned `Err(PluginLoadError::NotApproved)`.
  This pre-execution refusal halted execution before:
  - Checking the target architecture (`loader.rs:643`);
  - Creating any private snapshot directory or file (`image.rs:50`);
  - Acquiring filesystem sharing locks (`image.rs:61`);
  - Calling Windows `LoadLibraryExW` (`loader.rs:677`);
  - Executing any DLL code (`DllMain`, TLS callbacks, exports, `init()`).
  The candidate DLL was never mapped into memory by the OS.

## 3. Store design and image identity

The approval store (`crates/rustamp-plughost/src/approval.rs`) replaces manual
in-memory byte slice passing with an explicit, persistent cryptographic database.

For each approved plugin image, three identity properties are recorded and strictly verified:
1. **Whole-image cryptographic hash (`sha256`)**:
   - 32-byte digest represented as 64 lowercase hexadecimal characters.
   - Calculated across the entire file content using a pure-Rust, zero-dependency,
     clean-room implementation of FIPS 180-4 SHA-256.
   - *Why*: PE DLLs contain executable machine code, data, imports, exports, resources,
     relocations, and optional TLS callbacks. Any modification of even a single bit
     in any section or header completely changes the cryptographic digest.
     Timestamps and file paths are trivial to forge; only a whole-image cryptographic
     hash reliably guarantees that the exact byte sequence approved by the user
     is what will be staged and loaded.
2. **Target machine architecture (`machine = 0x014c`)**:
   - COFF machine header value (`IMAGE_FILE_MACHINE_I386` = 332 / `0x014c`).
   - *Why*: `rustamp-plughost32` is a 32-bit x86 process specifically dedicated
     to hosting 32-bit Winamp 2.x plugins (`specs/S09-plugin-abi.md` §0 lines 28–32).
     Explicitly validating the machine architecture prevents misconfiguration or
     attacks attempting to approve 64-bit (`0x8664`) or ARM binaries, which cannot
     be mapped into an i686 process and would otherwise fail later during OS loading.
3. **Image size in bytes (`size = u64`)**:
   - Total file length in bytes.
   - *Why*: Serves as a fast pre-check before reading and hashing large files,
     instantly catches truncated or appended files, and provides second-preimage
     defense-in-depth alongside the cryptographic hash.

## 4. Location, format and rationale

- **Where it lives**:
  A dedicated file named `plugins-approved.toml` located beside the user configuration
  file (which is managed by `crates/rustamp-core/src/config.rs`).
- **Format**:
  Standard TOML with an explicit schema header:
  ```toml
  # Rustamp plugin approval store v1
  version = 1

  [[plugins]]
  name = "in_openmpt.dll"
  sha256 = "c12217c18d1a3ebc5e2069b76c8c5750d52b11548e6583d7890b4d4f2066d1e4"
  size = 311296
  machine = 332
  ```
- **Rationale**:
  1. *Separation of concerns*: User settings in `config.toml` (volume, skin path,
     window geometry) mutate frequently during ordinary playback and UI operations.
     In contrast, plugin approval is an administrative security boundary. Isolating
     approvals in `plugins-approved.toml` prevents regular settings writes from
     inadvertently truncating, corrupting, or modifying the approval allowlist.
  2. *Auditability and transparency*: TOML is clean, human-readable, and editable.
     Users can inspect exactly which plugins and hashes are approved, or revoke an
     approval by deleting an entry with a text editor.
  3. *Fail-closed parsing*: Our pure-Rust parser enforces strict syntax validation.
     Any syntax error, unknown key, missing required field, or invalid hash/machine
     fails loudly, never silently ignoring lines or guessing intent.

## 5. How an image is approved

An image is approved via `ApprovalStore::approve_bytes(name, bytes)` or
`ApprovalStore::approve_file(path)`:
1. **Passive preflight validation**: The candidate bytes are checked with
   `check_pe_i386_dll(bytes)` to ensure they represent a structurally valid PE32 i386
   DLL with supported non-forwarded exports (`winampGetInModule2`, `winampDSPGetHeader2`,
   or `winampGetGeneralPurposePlugin`). Unparseable binaries, 64-bit binaries, or
   corrupt files are rejected before approval.
2. **Identity extraction**: SHA-256 digest is computed, file length is measured,
   and machine `0x014c` is asserted.
3. **Record creation**: An `ApprovalEntry { name, sha256, size, machine }` is created
   and recorded in the store.
4. **Atomic persistence**: `ApprovalStore::save_to_file(path)` writes to a unique
   temporary file in the target directory, syncs data to disk, and atomically
   renames it over the destination path.

## 6. How a changed file is refused

Even if an attacker or corrupted filesystem presents a file with the exact same
file name, directory path, file size, timestamps, and valid PE headers:
- `ApprovalStore::check_approval(&bytes)` computes the SHA-256 hash of the entire file.
- Even a 1-bit alteration in executable code or data results in a completely different
  SHA-256 digest.
- Because the computed hash is absent from the store's allowlist, the store returns
  `Err(ApprovalStoreError::NotApproved)`.
- When loading via `load_plugin_with_store`, `require_store_approval` maps this to
  `Err(PluginLoadError::NotApproved)` prior to snapshotting or OS mapping.
- Furthermore, during the loader's verified-byte snapshot pipeline (`PreparedImage::seal`),
  the source file is re-opened with `FILE_SHARE_READ` (denying write/delete sharing)
  and its full byte contents are compared again against the preflight bytes. Any modification
  between preflight and sealing returns `Err(PluginLoadError::ImageChanged)`.

## 7. Malformed store: refuse loudly, never silently trust

A corrupted or malformed approval store **never silently trusts or defaults to open**.
It fails loudly on any defect:
- Missing `version`: `ApprovalStoreError::Malformed("missing version header")`
- Unsupported version: `ApprovalStoreError::UnsupportedVersion(v)`
- Missing required fields (`sha256`, `size`, `machine`): `ApprovalStoreError::Malformed(...)`
- Invalid SHA-256 hex string (length != 64 or non-hex characters): `ApprovalStoreError::Malformed(...)`
- Non-i386 machine architecture (`machine != 0x014c`): `ApprovalStoreError::InvalidMachine`
- Conflicting duplicate entries: `ApprovalStoreError::Malformed(...)`
- Unknown keys or invalid syntax: `ApprovalStoreError::Malformed(...)`

When `load_plugin_with_store` encounters an `ApprovalStoreError` from a malformed store,
it maps to `Err(PluginLoadError::ApprovalStore(err))` and immediately halts.
Zero DLLs are loaded.

## 8. Preserved loader guarantees

All existing guarantees established by task P5 are strictly preserved:
1. **Passivity before execution**: Preflight and approval checks execute zero guest
   or plugin code (no TLS callbacks, `DllMain`, getter or `init()`).
2. **Snapshot sealing**: Source paths never reach `LoadLibraryExW`. The loader exclusively
   maps a byte-verified private copy in a locked directory namespace.
3. **Ancestor locking**: Canonical directory ancestors of the private copy remain pinned
   with non-sharing directory handles throughout the load.
4. **Target architecture**: 32-bit x86 process constraint (`cfg!(target_arch = "x86")`)
   and UI-thread host window requirement are preserved.
5. **Post-load validation**: Getter resolution, module version checks, host callback
   wiring, and UI-thread initialization remain unchanged.
6. **No weakening**: `load_plugin(path, approved_image, window)` remains completely
   intact; `load_plugin_with_store(path, store, window)` layers store verification
   without bypassing any snapshot or preflight check.

## 9. Tests and required VM work

All tests are headless, self-contained, and use synthetic in-memory test fixtures.
None call `LoadLibraryExW` on the host, open windows, or play audio.

### New headless tests in `crates/rustamp-plughost/src/approval.rs`:
1. `sha256_nist_standard_vectors`: verifies the clean-room SHA-256 implementation
   against NIST test vectors (empty string, `"abc"`, multi-block message).
2. `empty_store_approves_nothing`: verifies that a default or empty store rejects
   even structurally valid i386 DLL bytes with `NotApproved`.
3. `approved_image_is_accepted_and_matches_exact_identity`: approves valid i386
   bytes, checks that name/hash/size/machine match, and asserts `check_approval` succeeds.
4. `changed_image_is_refused_even_with_identical_size_and_headers`: mutates 1 byte
   in an approved PE binary (leaving size and headers identical); verifies immediate
   rejection with `NotApproved`.
5. `malformed_store_is_refused_loudly`: tests loud refusal on empty string, wrong
   version, missing fields, bad hex, non-i386 machine, and duplicate entries.
6. `approval_store_roundtrip_and_atomic_file_io`: tests serialization to TOML,
   re-parsing, atomic temporary file save, and disk reload.

### New headless tests in `crates/rustamp-plughost/src/loader.rs`:
7. `loader_with_store_approved_image_loads_to_arch_boundary`: proves that an approved
   image passes preflight and store approval, reaching the target architecture check
   on the x86_64 host without refusal.
8. `loader_with_store_changed_image_is_refused_before_loading`: proves that a modified
   DLL candidate is refused as `NotApproved` before any snapshot or OS loader invocation.
9. `loader_with_store_malformed_store_refuses_loudly`: proves that a corrupted or
   unsupported store refuses loudly with `PluginLoadError::ApprovalStore`.
10. `loader_with_empty_store_approves_nothing`: proves that an empty store refuses
    loading with `PluginLoadError::NotApproved`.

### What only the VM can prove

Headless unit and integration tests prove the cryptographic parsing, allowlist checks,
rejection paths, snapshot sealing, and anti-TOCTOU logic.
However, only the Hyper-V test VM (`rustamp-test`, running an i686 target with staged
`in_openmpt.dll` and interactive session) can prove:
- That Windows `LoadLibraryExW` successfully maps the snapshot DLL into a 32-bit address space;
- That dependent DLLs (`openmpt-mpg123.dll`) are resolved via `AddDllDirectory`;
- That Windows dynamic linking successfully executes `DllMain` and TLS callbacks;
- That the undecorated export `winampGetInModule2` returns a valid static `InModule` pointer;
- That real Win32 `WM_USER` / `WM_WA_IPC` messages are exchanged synchronously with
  the host window (`Winamp v1.x`) and receive correct responses (`IPC_GETVERSION` -> `0x2905`).

## 10. Files and line counts

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-plughost/src/approval.rs` | 991 | new: clean-room SHA-256, `ApprovalEntry`, `ApprovalStore`, parser/serializer, atomic I/O, 6 unit tests |
| `crates/rustamp-plughost/src/lib.rs` | 31 | +1: `pub mod approval;`, re-exports for store and `load_plugin_with_store` |
| `crates/rustamp-plughost/src/loader/image.rs` | 171 | +12: `require_store_approval` |
| `crates/rustamp-plughost/src/loader.rs` | 1517 | +116: `ApprovalStore` error variant, `load_plugin_with_store`, 4 integration tests |
| `docs/tasks/P7-approval-store.md` | 244 | this card |

No other crate was modified. No changes to `Cargo.toml`, `Cargo.lock`, or `THIRD_PARTY.md`.

## 11. Compile-only receipts

All commands run in `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave66-approval-store`:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo build --workspace` | exit 0 |

## 12. Verification checklist

- [x] Boundary maintained: only owned paths in worktree modified (`crates/rustamp-plughost/**` and `docs/tasks/P7-approval-store.md`).
- [x] No `cargo test` run, no test binary run, and no VM started.
- [x] No DLL or native library loaded.
- [x] No window opened, no audio played.
- [x] No download performed, no `cargo add`, no dependencies added.
- [x] No git writes, no commit made.
