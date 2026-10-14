# RFC 0004: Plugin-facing licensing split and the wire-type crate

Status: **Accepted** on 2026-10-14 by the integration lead, on the project
owner's decision to follow the primary recommendation of
[`docs/notes/L-licensing-plugins.md`](../notes/L-licensing-plugins.md).
Owner: Integration Lead.
Requires: RFC 0001 (core contracts), RFC 0003 (shared-memory rings).

The legal reasoning is **not** repeated here: it lives in the licensing note,
which records the FSF's plugin/linking/process-boundary positions with sources,
the precedents, the options and their consequences for every free source the
plan depends on. This RFC is the technical contract that implements the
decision, and it is what an implementer works from.

## 1. Decision

1. A new crate **`rustamp-proto`** holds every type that crosses the process
   boundary between `rustamp.exe` and `rustamp-plughost32.exe`.
2. **`rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc` and
   `rustamp-plughost` are licensed `MIT OR Apache-2.0`.**
3. **Every other crate stays `GPL-3.0-or-later`**, including `rustamp-core`,
   `rustamp-decoders`, `rustamp-audio`, `rustamp-skin`, `rustamp-gfx`,
   `rustamp-ui`, `rustamp-app` and the rest.
4. The GPL crates that contain **only Rustamp's own code** carry a
   **GPL-3.0 §7 additional permission** (a plugin exception) so that combining
   them with modules that speak the documented plugin ABI or the documented IPC
   protocol needs no further grant. Crates that carry third-party GPL code
   (e.g. `rustamp-decoders`, if it ever ports GPL sources) must **not** carry it:
   only a copyright holder may grant the exception for their own code.

## 2. Why this shape

The process that loads a plugin DLL is `rustamp-plughost32.exe`, not the
player. Making the host's license permissive removes the copyleft from the
plugin edge structurally, for plugins of every license, including GPL-2.0-only
ones, which are incompatible with GPL-3.0. The §7 permission on the player
covers the player↔host edge (where the shared-memory sentence in the FSF FAQ is
the honest weak point) for our own code. The two instruments cover each other's
weaknesses: a license is durable, a §7 permission is not; a §7 permission
reaches the GPL player, a permissive host license does not need to.

## 3. The contract change: what moves into `rustamp-proto`

Moved **out of** `rustamp-core`, which then re-exports them so that no existing
caller changes:

| Item | Notes |
|---|---|
| `vis`: `VisFrame`, `VIS_CHANNELS`, `VIS_SAMPLES` | also used by the RFC 0003 ring layout |
| `command::Command`, `command::SelectOp` | the whole enum, since the wire carries it |
| `event::Event` and the types it carries | only the variants that are actually sent over IPC stay in `proto` if the split is clean; otherwise move the whole type and re-export |
| `state::PlayerState`, `state::StreamInfo`, `state::RepeatMode`, `state::WindowId` | `PlayerState` is sent in full |
| `playlist::EntryId`, `playlist::MediaLocation`, `playlist::SortKey` | the subset the protocol refers to |

Rules:

- `rustamp-proto` has **no internal dependencies** and no GPL-licensed
  dependency. It may depend on `serde`/`postcard`/`thiserror`.
- `rustamp-core` depends on `rustamp-proto` and re-exports the moved items at
  their **existing paths** (`pub use`), so `rustamp_core::vis::VisFrame`,
  `rustamp_core::command::Command` and so on keep resolving. This is the
  acceptance test for the whole refactor: **no other crate needs a source
  change** beyond its manifest.
- `rustamp-ipc` and `rustamp-plughost` must **stop depending on
  `rustamp-core`**; they depend on `rustamp-proto` and `rustamp-plugin-abi`.
  `rustamp-app` and the player crates may keep depending on both.
- **No wire change.** `PROTOCOL_VERSION` stays `1`, the ring header layout and
  the postcard encoding are unchanged. This is a source-level refactor.
- The RFC 0003 compile-time layout asserts (`RING_HEADER_SIZE`,
  `VIS_SLOT_SIZE`, the x86/x86_64 offset checks) must keep passing on both
  targets. That is the reason the types are **extracted, not duplicated**: one
  definition is the only safe way to keep the byte-identical invariant.
- `rustamp-proto` and the other permissive crates must build for
  `i686-pc-windows-msvc` as well.

## 4. License metadata and files

- Per-crate override of the workspace default:
  `license = "MIT OR Apache-2.0"` in the four permissive crates' manifests,
  replacing `license.workspace = true`.
- Canonical, unmodified `LICENSE-MIT` and `LICENSE-APACHE` at the repository
  root, with a short note in each permissive crate pointing at them. The texts
  must be the canonical ones — take them from a dependency that ships them
  locally (e.g. in the cargo registry) rather than retyping them.
- `PLUGIN-EXCEPTION.md` at the repository root holding the §7 additional
  permission text, referenced from the crate root of each GPL crate that carries
  it. The text is drafted in the licensing note (§4.1) and must state:
  the permission is under GPL-3.0 §7 (or any later version at the recipient's
  option), it covers combination through the documented plugin ABI and the
  documented IPC protocol only, it does not apply to third-party GPL material in
  the covered work, and it requires the interfaces to stay unmodified.
- `THIRD_PARTY.md` gains a **"Licensing of Rustamp itself"** section: the split,
  which crates are permissive, the §7 exception and its scope, and the statement
  that the permissive crates contain no third-party code.
- `docs/PLAN.md` §1/§2 and `CLEANROOM.md` §1 (both currently say the project is
  GPL-3.0) note the split and point at this RFC.
- `CONTRIBUTING.md` and `.github/pull_request_template.md` gain an
  **inbound = outbound** statement: a contribution to a crate is licensed under
  that crate's license. This must land **before** the first external
  contribution, because after that relicensing needs every contributor's consent.

## 5. Non-goals

- No protocol, ABI, wire-format or behaviour change.
- The player does not become permissive, and its copyleft is not weakened.
- No plugin is bundled or redistributed; if a specific plugin is ever shipped,
  its license is a separate, per-plugin decision.
- The clean-room rules are unchanged.

## 6. Acceptance

1. `cargo test --workspace --all-features` green with the **same test count** as
   before the refactor.
2. `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
   `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
   `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` green.
3. `cargo xtask ring-cross` still passes all four combinations.
4. **No crate in the permissive set depends, directly or transitively, on a
   GPL-licensed crate.** Prove it from the dependency graph, not by inspection.
5. `rustamp_core::vis::VisFrame`, `rustamp_core::command::Command` and the other
   moved paths still resolve, and no crate outside the owned paths needed a
   source edit to keep compiling.
6. The license files are the canonical texts and `PLUGIN-EXCEPTION.md` names the
   interfaces and the GPL version it applies under.

## 7. Errata (2026-10-14, after the implementation)

Written by the integration lead; these correct the text above, which stays as
the record of what was decided.

- **§3 lists `state::RepeatMode`, which does not exist.** The codebase carries a
  `TransportState` and a `repeat: bool` field; the wire carries
  `TransportState`. Nothing was invented: the implementation moved what is
  actually there. Read the §3 table as "the state types the protocol uses".
- **§4 says `CLEANROOM.md` §1 states the project is GPL-3.0.** §1 is the source
  table; the split note was added there as intended, but the sentence was
  wrong about what §1 contains.
- **The §7 exception is attached to the repository, not yet to each crate
  root.** `PLUGIN-EXCEPTION.md` exists at the root and is referenced from
  `README.md`; the per-crate reference was left out of the first implementation
  because those crates were outside the task card's owned paths. The reference
  is being added to each GPL crate root as a follow-up.
- **Two crates are deliberately excluded from the exception**, and the reason
  is narrower than "third-party code": `rustamp-decoders` (third-party decoder
  material) and `rustamp-skin` (webamp MIT tables). Only the copyright holder
  of a piece of code may grant the exception for it, so the exclusion list must
  be reviewed whenever third-party material enters a crate.
