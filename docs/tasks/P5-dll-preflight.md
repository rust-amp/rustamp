# P5 — Verified plugin image loading and pre-execution refusals

## Scope and provenance

- Boundary: write only owned files in
  `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave57-codex-plughost`.
  Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
  `D:\code\rustamp-refbin` was read or written.
- Owned paths: `crates/rustamp-plughost/**` and this card only.
- Sources: worktree AGENTS/CONTRIBUTING/CLEANROOM/HANDOFF/PLAN, P3/P4, S09,
  existing clean-room implementation/tests and Windows platform API contracts.
  No Winamp source, SDK, disassembly or reference binary inspected; no copied
  third-party code, dependencies, lockfile, license or shared contract changes.
- `docs/tasks/W10-batch-review.md` is absent in this worktree. F2/F8 scope comes
  from the supplied finding descriptions and the cited implementation itself.
- Status: implementation and compile-only checks; **no runtime pass claimed**.
  This supersedes P4's original-path loading and machine-header-only description.

## Mechanism (F2)

Use a **copy of the verified bytes**, not a last-moment timestamp/identity check:

1. Canonicalize the source or fail; there is no relative-path fallback, lossy
   Unicode conversion or removal of the extended Windows path prefix.
2. Read and passively preflight the bytes. Compare the entire image with the
   caller's independently approved byte image.
3. Re-open the canonical source with `FILE_SHARE_READ` only (no write/delete
   sharing). Re-read and compare every byte with the preflight image. A changed
   source returns `ImageChanged`, even with identical machine/header/length.
4. Exclusively create a unique private snapshot directory/file. Pin its canonical
   ancestors with directory handles denying write/delete sharing, write the
   verified bytes, close the writer, re-open a read-only/no-write/no-delete-share
   handle and compare every byte again. Fail closed on any open/compare failure.
5. Pass **only this absolute snapshot path** to `LoadLibraryExW`, keeping all
   locks alive. Never load the original source path. Unique snapshot paths avoid
   reusing an earlier mapping of the source pathname.
6. Keep snapshot/source/directory handles for process lifetime once mapped,
   including after a subsequent ABI refusal, consistent with the existing
   input/general no-FreeLibrary policy. Unmapped snapshots are cleaned up.

Why: a final source identity check alone leaves a check/load gap. Copying the
actual verified bytes decouples the load from later source-path replacement;
locking the snapshot and its ancestors protects its path throughout the load.
The writer-to-reader transition is followed by a complete byte comparison,
not assumed safe. A mismatch never reaches the loader.

Dependency resolution uses `AddDllDirectory(source_parent)` and
`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_USER_DIRS |
LOAD_LIBRARY_SEARCH_SYSTEM32`, not legacy CWD/PATH search. The temporary cookie
is removed after mapping. Dependencies remain a separate trust decision.
`LoadedPlugin::path()` describes the canonical **source**; the mapped filename
is the private snapshot's `plugin.dll`.

## Checks and execution boundary (F8)

All of the following precede `LoadLibraryExW`, so a refusal here runs neither
TLS callbacks, DllMain, plugin getters nor init:

- Required canonical source path/read; MZ, declared PE offset/signature and full
  COFF header; machine exactly `0x014c` (actual rejected value in the diagnostic).
- Executable/DLL characteristics; PE32 optional magic `0x010b` (reject PE32+
  even if COFF claims i386); bounded optional/header/image/section tables;
  1..=96 sections; raw/virtual ranges in bounds without overlap/overflow.
- Entry RVA, if nonzero, backed by executable section bytes.
- Bounded export tables, an exact supported undecorated input/general getter,
  unique/sorted bounded NUL-terminated names, valid ordinals, file-backed
  executable getter RVA; no forwarded getter. Names longer than 4095 bytes
  are refused as an own conservative policy, not an S09 ABI limit.
- Complete byte equality with explicit `approved_image`; supported, structurally
  valid but unapproved i386 code is refused as `NotApproved` before execution.
- Windows/i386 host, nonzero host HWND, source revalidation, snapshot equality,
  namespace/file locks, path encoding and search-directory setup.

The loader API is now `load_plugin(path, approved_image, window)` (still unsafe).
Approval is a required full-byte allowlist input from an independent trusted
decision. The loader does **not** create approval by reading the candidate.
The VM-only acceptance runner explicitly trusts the owner-approved staged
fixture; it is not a production approval database or a general path allowlist.

Only **after** Windows has initialized the approved DLL/dependencies can we
check the actual export address, call the getter, check its returned non-null
module pointer and dynamic version, read its description, fill fields and call
init. Unsupported module versions/null getters are post-execution ABI refusals,
not a claim that DllMain was prevented. S09 permits runtime-filled module data;
there is no generally sound static version check for arbitrary getter code.

## Residual boundaries (do not overclaim)

- No ordinary source-path check/load race remains for the main DLL under Windows
  file-sharing/namespace semantics: the loader uses the checked, locked copy.
  A source change before sealing is refused; later source-path changes cannot
  alter the snapshot being mapped.
- This is **not a sandbox or malware detector**. A malicious image that someone
  explicitly approves can still run at TLS/DllMain/getter/init. Automatically
  passing candidate bytes as approval would defeat the trust gate and violate
  the unsafe API contract. The project still needs a production approval store.
- Not a defense against same-principal process injection, kernel/raw-volume
  writes, drive-namespace manipulation, or a malicious pre-existing writable
  mapping into the snapshot during the writer/reader transition. Those require
  OS isolation/private ACL policy beyond share locks. Do not claim zero risk
  against an attacker already controlling this process/security principal.
- Imports, TLS metadata, CPU instructions, signatures and the dependency graph
  are not fully validated here. Dependencies/previously mapped modules and
  concurrent process-wide USER_DIRS changes are not byte-pinned. An OS load
  failure can follow dependency initialization. Delayed/plugin-initiated loads
  remain the plugin's responsibility, not guaranteed by the temporary cookie.
- Snapshot/module filename and sibling-resource behavior differ from loading
  in place. Real in_openmpt dependency/resource compatibility needs VM acceptance.
  Retained snapshots leave temp files after process exit; no automatic stale-file
  cleanup is added. Pinning ancestors may fail on restrictive installations;
  such failures are explicit I/O refusals, never unprotected fallback loads.

## Tests and required VM work

Written/compile-checked only; **none executed in this lane**:

- Five new passive-parser tests: structural input/general positives (not actual
  loadable fixtures), wrong machine/lying PE32+, every truncated DLL prefix,
  malformed sections/tables/ordinals, non-executable/forwarded/decorated getter,
  and unsorted names that a linear scan could falsely accept before Windows'
  actual name lookup.
- Five new loader tests: wrong-architecture/missing-export refusal through the
  public API; valid-looking unapproved i386 code refusal; same-size source-byte
  mutation between preflight and sealing; byte-identical sealed copy with
  write/delete/ancestor-rename denial and cleanup; disappearance after preflight.
  These are headless, use synthetic files and never call an OS DLL loader.
- Existing malformed/missing-file tests kept, with unique fixture filenames.
- One new ignored, VM-gated **headless real mapping** test:
  `approved_real_dll_maps_headlessly_from_verified_snapshot`. It requires i686
  and the owner-approved fixture under `RUSTAMP_TESTPLUGINS`, uses the same
  production snapshot/map primitive and requires a non-null module. No host
  window, getter, plugin init/quit or audio; Windows still runs TLS/DllMain.
- Existing ignored, VM-gated real i686 `in_openmpt` acceptance now supplies explicit
  approved fixture bytes and checks `GetModuleFileNameW`: the actual mapped path
  differs from the source, all mapped file bytes match, and the copy stays
  write-locked. It still checks getter/init/IPC/quit as before.

The orchestrator must run all headless tests in the Hyper-V VM, including the
ignored headless mapping test in its **i686** build, and rerun the ignored
real-DLL acceptance with approved in_openmpt and its dependencies staged. Only
the real VM tests prove that a valid DLL still maps/initializes with this policy.
Synthetic positives, Clippy and a default-target build do not.
The existing acceptance creates a host window and therefore is not headless;
only the VM may execute it. No valid-DLL execution is claimed here.

## Compile-only receipts

Final commands, all run in the owned worktree:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features --offline --locked --target-dir target -- -D warnings` | exit 0 |
| `cargo build --workspace --offline --locked --target-dir target` | exit 0 |

These are default-target compile checks, not i686 execution or a VM test pass.
Windows loader internals also compile on the default Windows target; the runtime
entry points still refuse unless the executing process is i386.
Eleven new test functions and the strengthened existing acceptance were
compile-checked only. Earlier formatting iterations and one constant-assertion
Clippy diagnostic were corrected before these final successful receipts.

| Authored file | Final lines | Delta |
|---|---:|---:|
| `crates/rustamp-plughost/src/loader.rs` | 819 | +336 / -107 |
| `crates/rustamp-plughost/src/loader/image.rs` | 158 | new |
| `crates/rustamp-plughost/src/pe_preflight.rs` | 576 | +386 / -1 |
| `crates/rustamp-plughost/tests/dll_acceptance.rs` | 195 | +41 / -1 |
| `docs/tasks/P5-dll-preflight.md` | this card | new |

Final worktree-local `git status --short` must show only those five paths (three
modified, two new); no other crate, root manifest or lockfile was edited.
No `cargo test` or test binary ran, no plugin/reference library was loaded,
no window was opened, no audio played, no download/dependency change occurred,
and no git write or commit was made. Builds remain in worktree-local `target`.
