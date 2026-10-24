# W20 — projectM 4.1.8 separate clean-room review and player-only bundle plan

**Date:** 2026-10-18. **Scope:** owner-approved step 2, documentation and verified
source acquisition only. No build or distribution is authorized by this card.
The owner selected projectM 4.1.8, LGPL-2.1-or-later, superseding the MilkDrop
recommendation (not the legal constraints) of `W20-vis-plugin-choice.md` §3.

## 1. Separate review required by CLEANROOM.md §7(4)

### 1.1 Policy decision, before any upstream implementation is read or copied

Read in full: `AGENTS.md`, `docs/tasks/W20-vis-plugin-choice.md` (especially
§§2.2, 3), `CLEANROOM.md`, `docs/notes/L-licensing-plugins.md`,
`docs/HANDOFF.md`, `docs/PLAN.md`, `CONTRIBUTING.md` and
`PLUGIN-EXCEPTION.md`. Also consulted: the current inventory and release tooling.

`CLEANROOM.md` §1 and PLAN §1 explicitly permit everyone to read projectM as a
free source; §7(4)'s **"LGPL, reference only; do not copy ... without a separate
review"** is the narrower copying gate. These provisions are read together:
reference reading is permitted; neither the source-table shorthand "copy
allowed" nor LGPL/GPL compatibility grants blanket approval to port its code.
This review records that distinction and the requirements for a future copying
decision; **it approves no particular copied line, file or port**.

Permitted reference learning from demonstrably free projectM-owned material:
its documented library API, build requirements, supported rendering/audio input
capabilities, preset formats, architecture and general algorithmic ideas.
Independently written FFI calls to a reviewed free API are not a port of its
implementation; generated bindings, inline functions, macros or examples copied
from headers still require provenance and license review. Renaming or translating
code, or closely following its implementation structure, is adaptation, not an
independent implementation or a way around §7.

The LGPL label on the project does not cleanse foreign material in its tree.
Do not open or adapt bundled Winamp SDK headers/docs, Winamp-derived frontend
code of unestablished provenance, Winamp source, reference binaries, disassembly
or the private research repository. A path being in a free project's archive is
not sufficient proof of a usable grant. Unknown-provenance content stays closed
pending a safe per-file classification; uncertain cases go to the integration
lead, not to an implementer reading them to find out. No claim is made that this
archive has been exhaustively classified or that all its files are LGPL.

### 1.2 What Rustamp must write independently

- A new classic `vis_` wrapper, not an adaptation of the legacy projectM Winamp
  frontend or an SDK sample. Derive its ABI declarations from exported, audited
  `specs/S09-plugin-abi.md` and the clean `rustamp-plugin-abi` crate, not `vis.h`,
  `wa_ipc.h`, Wasabi or other Winamp SDK material.
- Rustamp-specific lifecycle, audio conversion/feed, rendering-window/context
  ownership, configuration and preset selection, error handling and shutdown
  glue. Use Rustamp's exported specs for classic behavior; gaps are spec requests
  under CLEANROOM §§3–6, not answers to obtain from upstream's old Winamp code.
- Build/package integration, loader configuration and source/relink instructions
  for Rustamp's actual artifact. projectM's maintained library can remain a
  separately licensed dependency; copying its renderer into Rustamp is not needed
  for that approach and is not authorized here.

Reference learning is not evidence of Winamp 2.95 behavior: projectM is not the
reference product. No upstream internal Winamp implementation, expressive code,
comments or structure may be laundered into specs or an allegedly original port.

### 1.3 Evidence required before any later copying

For every proposed file/section: identify the exact upstream release/commit,
archive hash, source path and line/section range, destination path and copied or
adapted extent; establish its actual copyright holders, unchanged original
header, license grant and license text, including third-party and generated
origins. Missing/conflicting grants are a blocker, not covered by the top-level
LGPL label. Record an explicit per-copy review of destination GPL compatibility,
LGPL obligations, plugin-exception scope and the protected permissive closures.

Then retain copyright/license headers unchanged, add the required attribution,
record source/license/version/destination in `THIRD_PARTY.md`, list the copy in
the PR provenance checklist and obtain reviewer clearance under CLEANROOM §7
items 1–5 and §8. Changes to LGPL library files also need prominent modification
and date notices (§2(b)) and the applicable library-source obligations. Our GPL
§7 exception cannot be granted for upstream code. Approval of this general review
is not evidence that any individual line satisfies these requirements.

**Negative-access declaration:** no Winamp source, SDK header/doc, legacy Winamp
frontend implementation, reference binary or disassembly was opened. Nothing in
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was accessed; nothing in
`D:\code\Rustamp` was opened or written. No projectM implementation was read or
copied to produce this review. Later artifact inspection is limited to filenames,
release metadata, README/build metadata and explicitly selected license texts.

## 2. Verified acquisition receipt

Download expressly authorized by this task. Staging, outside every checkout:
`C:\Users\micro\AppData\Local\Temp\opencode\wave98-projectm-4.1.8-staging\`.
Only this staging directory and this card are owned write paths.

| Field | Observed value |
|---|---|
| Component/version | **libprojectM 4.1.8**, tag `v4.1.8` |
| Official upstream home | https://github.com/projectM-visualizer/projectm |
| Release | https://github.com/projectM-visualizer/projectm/releases/tag/v4.1.8 |
| Source artifact | `libprojectM-4.1.8.tar.gz` (uploaded release asset, **not** the automatic GitHub tag-source archive) |
| Exact download URL | https://github.com/projectM-visualizer/projectm/releases/download/v4.1.8/libprojectM-4.1.8.tar.gz |
| Size, downloaded and upstream | **1,309,526 bytes** |
| Downloaded SHA-256 | **`f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`** |
| Published upstream digest | `sha256:f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e` |
| Published metadata endpoint | https://api.github.com/repos/projectM-visualizer/projectm/releases/tags/v4.1.8 |
| Provenance | Official project organization/repository; release ID `404509882`, asset ID `615012180`, uploader `kblaschke` |
| Published at | `2026-10-06T09:01:03Z` |
| Download verified at | `2026-10-18T13:42:52.6947245Z` |
| Result | **PASS:** exact match to both freshly retrieved upstream digest/size and the SHA-256 recorded in `W20-vis-plugin-choice.md` §2.2 |

Staging contains the archive, raw `upstream-release.json`, `PROVENANCE.json` and
`SHA256SUMS`. Verification used `Get-FileHash -Algorithm SHA256` on the downloaded
file; metadata digest was checked before download and bytes checked afterwards.
No detached upstream signature was verified. This is authenticated upstream
release-metadata/byte agreement, not an independent signature or a source audit.

Archive inventory: **716 entries**, read without extracting a source tree.
Only these archive document contents were opened (all paths relative to
`libprojectM-4.1.8/`):

- `COPYING`: projectM Team copyright 2003–2009 and the explicit **LGPL version
  2.1 or, at the recipient's option, any later version** grant.
- `LICENSE.txt`: full GNU Lesser General Public License 2.1, February 1999,
  including §§0–16; §6 is the source for the duties below.
- `vendor/hlslparser/LICENSE`: MIT, Unknown Worlds Entertainment, Inc.,
  copyright 2013–2014.
- `vendor/projectm-eval/LICENSE.md`: MIT, The projectM Team, copyright 2023.
- `README.md`: library/rendering, licensing and frontend status only. Lines
  86–87 classify ALSA/XMMS/**Winamp**/JACK as legacy 2.x sources, **unmaintained
  since 2012**; lines 167–169 describe PCM analysis and OpenGL output. The license
  section warns that other components may carry different licenses.

Reading these license documents is not importing implementation code. No archive
source/header, legacy frontend, preset or shader contents were opened. Vendor
grants above are document-level evidence, **not exhaustive per-file clearance**.
The release notes pin the vendored projectm-eval update to
`e8c311e86f4d8c082e96e02b6a33525034eee6d4`; a later build must inventory its
actual selected dependencies rather than silently substituting a package-manager
version. Preset collections/textures and additional dependencies require their
own distribution-rights review; they are not covered automatically by libprojectM's
LGPL grant and were not separately acquired.

## 3. Player-only bundling plan — proposal, not implementation

### 3.1 License split and concrete package location

Keep libprojectM **LGPL-2.1-or-later**, as a separately licensed shared library;
the player remains GPL-3.0-or-later. LGPL §6 permits a compliant linked work under
the application's terms. LGPL §3 also demonstrates GPL-3.0 compatibility, but
conversion to ordinary GPL is **not** the proposed shipping route: it would not
extend Rustamp's plugin exception to third-party GPL code and would require a
fresh review of that edge. Neither GPL compatibility nor the owner's component
choice authorizes relaxing the permissive host rules.

Proposed package layout, subject to the owner's build decision:

- Put the new plugin and its versioned, same-bitness libprojectM runtime in
  **`player/Plugins/projectM/`**, with proposed wrapper name
  **`vis_projectm.dll`**. The actual upstream DLL basename(s) must be frozen from
  the approved build; do not guess them in a whitelist. This is a player-bundle
  component, not a second permissive-host payload.
- Put the unmodified LGPL full text in **`player/LICENSE-LGPL-2.1`**. Extend
  **`player/THIRD_PARTY.md`** with projectM/vendor notices and the exact release's
  source/relink instructions; record all component paths and hashes in manifests.
  Make source/relink downloads available with that same release (§3.4).
- **No projectM library, wrapper, headers, assets, LGPL notices or native
  dependency enters `host/`**, its standalone artifact or any of the five
  protected crates' normal dependency closures (optional/target features included).
  No host `build.rs`, import library, direct link or projectM-specific mandatory
  runtime dependency is acceptable. Do not move/copy the plugin into `host/Plugins`
  to simplify discovery, and do not relicense the host or weaken its audit.

A generic permissive host can load a separately licensed plugin selected by a
user/player without having that plugin as a host build dependency. If the classic
wrapper route is approved, the plugin/library remain the **player-only optional
component**, loaded through the generic plugin interface by explicit path. That
runtime loading is not a claim that the loaded library is permissive. Packaging
alone does not establish the separation: future review must verify dependency
edges, loader behavior and independent host-only operation. If integration needs
projectM in the host's normal/mandatory closure, **stop and redesign**, not simply
relocate its files. The LGPL allows dynamic loading in general; **our own**
permissive-only package/closure policy is why it cannot ship in `host/`.

### 3.2 Strict release-tooling whitelist consequences

Current `xtask/src/package.rs:1707–1794` is a closed, exact layout:
`audit_package_layout` calls `audit_directory` with **no subdirectories** allowed
under either `player/` or `host/`. The player file list currently allows only the
three player/resident executables, optional bundled plughost, GPL license,
plugin exception, notices, manifest and quickstart. Dropping in `Plugins/` or an
LGPL license file will therefore fail `cargo xtask package --check` today.

Future approved work must add **only** the player-side `Plugins/projectM` subtree
and `LICENSE-LGPL-2.1`, with exact nested file allowlists for the frozen wrapper,
projectM runtime and reviewed dependencies/assets. No blanket `*.dll`, arbitrary
directory or arbitrary plugin allowance. Update the staging/copy logic,
player/top manifests (license paths, versions, hashes and architecture), generated
notices/quickstart and their exact-text audit expectations together. Model both
player-with-host and player-only modes explicitly: a 32-bit plugin is not usable
without the host, so omit it or clearly require the separately installed host;
host-only must remain independent and unchanged.

`audit_host_dir` (`package.rs:805–849`) rejects copyleft/MPL text in host notices
and unexpected executables; exact host layout whitelisting also excludes an
extra plugin/runtime. Its token scan alone is not a native-DLL license proof.
Keep those guards and the all-feature, both-target permissive closure checks;
extend future native artifact/dependency auditing rather than rely on a Cargo
license-metadata pass to find FFI libraries. Add regression coverage for host-only
purity, missing notices/source material, unexpected files, wrong-bitness DLLs and
all approved package modes. None of that tooling or coverage is changed/run here.

### 3.3 The new `vis_` wrapper and the architecture/build decision

projectM 4.1.8 is a maintained **library**, not a maintained Winamp plugin.
Write **`vis_projectm.dll`** here from scratch. For a classic-host path it must be
an **i686/x86 DLL**, exporting `winampVisGetHeader`, with the S09 §5 classic
`0x101` header and **2360-byte** module layout. Use `rustamp-plugin-abi` for
declarations, module enumeration and config/init/render/quit callbacks; manage
the parent handle, sample rate/channels and requested spectrum/waveform rows
(each up to 2 × 576 bytes) through that clean contract. Do not add a 5.x SDK
dependency just because upstream's old frontend used it.

Implement the reviewed projectM C-API FFI bridge, audio adaptation (waveform/PCM
and channel/sample-rate handling), preset/config state, OpenGL context/window
lifetime, resize, render scheduling and error/quit cleanup as original Rustamp
glue. projectM does its own PCM analysis; do not mislabel classic spectrum bytes
as PCM or invent a byte sign/scale convention. Resolve it from the current audited
S08/S09/S11 specs and file spec requests for any unresolved behavior. Do not
reuse upstream's abandoned Winamp frontend or read it to fill those gaps.

The host remains the existing i686 process; projectM and all native libraries
loaded **by this plugin** must match that architecture, with no projectM linking
into the host executable. The player stays x86_64 and never loads that i686 DLL.
The proposed plugin/native-library build is additional build work, not an
already-approved Cargo dependency or a verified toolchain capability. A native
x64 player integration would be a different owner-approved frontend plan, not
an interchangeable classic `vis_` DLL. No new helper process or contract change
is assumed; any shared-contract change needs its own RFC.

### 3.4 LGPL-2.1 §6 release duties and source/relink delivery

These duties attach to **distribution**, not this unbuilt staging download.
They also apply when the linked work is a wrapper DLL; using a DLL suffix does
not remove the combination/relink conditions. Based on the actual release's
`LICENSE.txt` (§§0–6), the release checklist is:

1. **§6 opening condition:** distribution terms must allow modification for the
   customer's own use and reverse engineering for debugging those modifications.
   No installer/EULA/no-reverse-engineering restriction may contradict this.
2. **§6 notices:** give prominent notice with each copy that projectM is used and
   that the library **and its use** are covered by LGPL; supply the **full LGPL
   2.1 text**, not just an SPDX identifier or URL. If the program displays
   copyright notices while running, include projectM's copyright and a direction
   to the included license there too. Preserve upstream copyright, warranty and
   license notices (§1); identify library changes/dates (§2(b)).
3. **Choose and actually fulfill a §6(a)–(e) route.** Recommended: a replaceable
   shared projectM DLL satisfying **§6(b)**, plus a matching source/relink kit
   published beside the release (**§6(d)** equivalent access). §6(b) requires
   runtime use of a library copy on the user's system and proper operation with
   a user-installed **interface-compatible modified library**. Do not statically
   inline/copy projectM code while claiming this route, or lock replacement with
   hashes/signatures/path rules. Verify replacement in the later VM acceptance
   run. Shipping a projectM DLL itself still requires its complete corresponding
   source under **§4**; dynamic linking does not waive that obligation.
4. **Relink/rebuild obligation (§6(a), including the executable paragraph):**
   provide complete corresponding machine-readable library source **including
   every change actually used**, and the complete machine-readable "work that
   uses the Library" as **object code and/or source code** sufficient to modify
   the library and relink the resulting work. Include needed data and utility
   programs, interface definition files, build/install scripts, patches and
   exact build/link instructions. For a static link, supply usable relinkable
   objects and/or genuinely sufficient application/wrapper source and tooling;
   a tarball of libprojectM alone is not sufficient. §6(b) is an alternative
   mechanism, so dynamic distribution does **not automatically require** a
   separate application object-file bundle; Rustamp's GPL source duties remain.
5. **If choosing a written source offer, §6(c):** accompany the work with a
   written offer **valid for at least three years**, to give the **same user all
   §6(a) materials**, at no more than the cost of performing distribution. Name
   an actual fulfillment contact/location and archive the exact materials so it
   can be honored. A generic "upstream source is on GitHub" or only a projectM
   source link is not that offer. **§6(d)** instead permits equivalent access to
   all those materials from the same designated download place; **§6(e)**
   requires verifying that the user already received them, not assuming it.

The release-specific source/relink kit (whether directly delivered, equivalently
hosted under §6(d), or promised under §6(c)) must identify and contain:

- The **exact** projectM source represented by the shipped binary, this receipt's
  release/hash, all local patches/modification dates and native dependency source
  and notices required to rebuild that configuration; source for modules actually
  included, associated interfaces and build/install scripts (§0). Preserve the
  actual vendor revisions, including projectm-eval, rather than a moving URL.
- Original Rustamp wrapper/FFI/frontend source at the release commit, required
  non-system link inputs (or sufficient rebuild sources), configs, resources and
  build/link scripts; Rustamp application source when it is the linked client,
  plus its separate GPL-3.0 Corresponding Source obligations. Any applicable
  exception scope must remain limited to Rustamp's own code.
- Compiler/toolchain/CMake versions and flags, target architecture, features,
  runtime/DLL search layout, dependency versions/hashes, reproducible build and
  relink/replacement instructions. Actual object files must be supplied if the
  chosen delivery cannot otherwise reproduce and relink the linked client.
  The §6 system-component exception is narrow; do not treat every external
  proprietary library as a system library. Conflicting non-system dependency
  restrictions block distribution (§6 final paragraph).

Keep that kit and its checksums with **each binary release**. This staging
tarball is provenance input, **not** the finished corresponding-source offer for
an as-yet nonexistent Rustamp/projectM build. No public source hosting/contact or
three-year fulfillment commitment is asserted to exist by this card.

### 3.5 Notices and third-party inventory changes required later

Before any shipped build, add a **native bundled-component** entry to root
`THIRD_PARTY.md`: libprojectM **4.1.8**, upstream/release URL, archive digest,
**LGPL-2.1-or-later**, copyright holders, consuming wrapper/player paths, shipped
DLL paths/architecture, exact selected dependency revisions, modifications and
source/relink delivery. If anything is actually copied/ported, add separate
per-copy source/destination/version entries and retain original headers as §1.3
requires. Do not misclassify an FFI library as a copied Rust source port or insert
it into the generated Cargo.lock block without a real Cargo package.

Update the "Projects we expect to draw from" status to distinguish this selected
native library from the earlier butterchurn/native-port roadmap. Inventory every
actual native dependency and redistributed preset/texture separately, with its
own grant and destination; the two checked MIT vendor licenses already require
their copyright/permission/warranty notices if included. Their document-level
grants do not pre-clear every file or optional dependency.

Generated **`player/THIRD_PARTY.md`**, the root/package inventory, release source
kit and runtime copyright/about notices must agree. Add projectM's explicit
or-later grant, retained notices, full LGPL 2.1 text (also the named player license
file), MIT texts and component-specific copyright notices for included vendors,
and accurate source/relink instructions. Extend manifests to enumerate the new
license files and native components. Keep **`host/THIRD_PARTY.md` and host-only
licenses/inventory permissive and projectM-free**; do not solve this by copying
the repository-wide LGPL entry into the standalone host notices. Existing GPL,
MIT, Apache, MPL and other obligations remain; this adds, not replaces, them.

## 4. Decisions and remaining gates for the owner

Component/version/license selection is **already decided**. Remaining decisions:

1. Accept this separate review as the projectM gate record, with **no blanket
   code-copy approval**; authorize only a separately reviewed library integration.
2. Approve the **player-only** layout above and a narrow X10 whitelist/manifest
   change, including plugin discovery by explicit player-bundle path and what
   player-only-without-host packages should contain. Host purity is not optional.
3. Decide the build: new i686 `vis_projectm.dll` + i686 shared projectM/OpenGL
   runtime via the classic host, or a separately planned native x64 frontend;
   select toolchain/features/CRT policy and review every actual native dependency
   and optional preset/texture license. The old frontend is not the shortcut.
4. Choose **§6(b) + §6(d)** as recommended, or specify another compliant route;
   assign release-source hosting, retention and fulfillment ownership. If §6(c)
   is chosen, explicitly accept its at-least-three-year offer responsibility.
5. Approve the future inventory/notices/LGPL-full-text work and implementation
   task, followed by independent license/closure/package/relink review and VM-only
   interactive acceptance. None is authorized or claimed complete by acquisition.

This is a policy/compliance review, **not legal advice**; the plugin/process
boundary caveats in `L-licensing-plugins.md` remain. No legal certainty or
copyright-holder exception in third-party code is invented here.

## 5. Scope and boundary receipt

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave98-projectm`.
  Initial `git -C <worktree> status --short`: **empty**.
- The sole repository write is **`docs/tasks/W20-projectm-review.md`**; other
  writes are only the four acquisition artifacts in the staging directory above.
  No writes or reads in the canonical `D:\code\Rustamp`, research or refbin trees.
- No bundled file, dependency, Cargo manifest/lock change, packaging-code change,
  upstream implementation copy, build, install, execution, VM, test, git write
  command or commit. In particular **no `cargo test`** was run, per task scope.
  No fmt/clippy/build verification is claimed for this documentation-only task.
- Final `git -C <worktree> status --short` must show only:

  ```text
  ?? docs/tasks/W20-projectm-review.md
  ```

  This worktree status proves the visible repository delta, not a global
  filesystem-write audit. The orchestrator can independently check its canonical
  tree; this lane does not enter it to do so.
