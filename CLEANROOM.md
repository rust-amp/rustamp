# Clean-room policy

This policy is binding for everyone who contributes to Rustamp, human or agent. It is derived from section 1 of [docs/PLAN.md](docs/PLAN.md). If this file and the plan disagree, stop and raise it in an RFC.

Rustamp re-implements a product whose source code is not free. We keep Winamp's observable behavior and file formats, and we never copy its code, expression or internal structure. The rules below keep that line clear.

## 1. Source table

| Source | Status | Who may read it |
|---|---|---|
| alexfreud/winamp (Winamp **5.x** source) | must not be copied | **Dirty room only** |
| Winamp 2.95 binary and classic plugin DLLs, plus disassembly (Ghidra headless) | interoperability reverse engineering | **Dirty room only** |
| Winamp SDK headers and docs | treat as dirty | Dirty room only. The ABI goes into specs as *facts* (sizes, offsets, constants, call order) |
| foobar2000 | analyze behavior and architecture, copy nothing | Dirty room writes design notes |
| WACUP changelog (https://getwacup.com/changelog/) | inspiration only: feature ideas, plugin-compatibility quirks, modernization ideas. No code, assets or verbatim text; WACUP is closed source | Everyone |
| DeaDBeeF, webamp, butterchurn, WDL/EEL2, libopenmpt, projectM (LGPL, reference only) | copy allowed, with attribution | Everyone |
| MilkDrop 2.25c source (geisswerks.com, `milkdrop_225c_src.zip`): files under `vis_milk2/` that carry the Nullsoft BSD-3 header | copy allowed, with BSD notice kept and attribution | Everyone. **Exception:** the bundled Winamp SDK headers in that zip (`Winamp/`, `Wasabi/`, `Agave/`, `vis_milk2/vis.h`, `api.h`) and any file without a BSD header are dirty. Check the `ns-eel2/` license separately before use |

"Dirty" sources are those that may only be read in the dirty room. "Free" sources are open-licensed projects that may be read and, with attribution, copied.

Rustamp's **own** license is split by [RFC 0004](docs/rfc/0004-plugin-licensing.md) and its §7 erratum: the player stays GPL-3.0-or-later, while the five permissive crates (`rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost`) are MIT OR Apache-2.0. The last is only the media-host protocol framework, not a decoder resident. The GPL crates in the scope of [PLUGIN-EXCEPTION.md](PLUGIN-EXCEPTION.md) carry a GPL-3.0 §7 plugin exception for our own code only. This changes nothing in this file: the source table above and the copying and attribution rules in section 7 apply to every crate, whatever that crate's own license is.

## 2. Roles

**Dirty room (R-agents, research repo).** They read the dirty sources and write specs. They run and automate Winamp 2.95, capture screenshots and timings, and disassemble binaries. The research repo is `rustamp-research`. It is private and is never pushed to `rustamp`.

**Clean room (implementers).** Implementation agents work in their own git worktree of `rustamp`. They work only from task cards, exported specs in `specs/` and free sources. They are not given access to the research repo, to Winamp source, SDK material or disassembly, and they do not read them.

**Orchestrator and integration lead.** These roles stay clean too. They write task cards from specs and merge PRs, but they must not read dirty sources to resolve an implementation question. If a question needs dirty knowledge, it goes back to the dirty room as a request for a spec.

## 3. Spec pipeline

1. **Dirty-room agent** studies the 2.95 binary and dirty sources and writes a draft spec in private.
2. The draft lives in `rustamp-research`.
3. **Spec Auditor** reviews the draft for copied code, transliterated pseudo-code, source comments and distinctive internal identifiers. It removes or rewrites anything that fails.
4. Only after the audit is the spec exported to `specs/S-xx-name.md` (or `specs/D-xx-name.md` for D-notes), with the provenance header described in [specs/README.md](specs/README.md).

Specs are behavior, not implementation. The auditor is the gate; nothing reaches `specs/` without its audit date in the header.

## 4. What a spec may and may not contain

**A spec MAY contain:**
- Observable behavior, described in prose or tables.
- Timings, distances, frame rates, animation speeds, thresholds.
- Pixel coordinates, sprite rectangles, window sizes and layout deltas.
- Constants and default values.
- State machines and call order.
- ABI layouts as facts: field sizes, offsets, versions, constants.
- Names that interoperability requires, such as `In_Module`, `WM_WA_IPC` and the IPC message numbers. These are the public contract that plugins and external tools depend on.

**A spec MUST NOT contain:**
- Source code, in any language.
- Pseudo-code transliterated from the source or disassembly. Describe the behavior in plain words instead.
- Comments copied or paraphrased from the source.
- Distinctive internal identifiers: private function, variable, class or file names from Winamp's own code that are not part of the public interface.
- Disassembly listings, or instruction-level descriptions that map one-to-one onto machine code.

When in doubt, state the observable fact ("the seek bar moves 1 px per 2.5 s of playback at 1x") and leave out how it was computed.

## 5. Implementers never see dirty sources

- Implementation agents receive only the task card, the exported specs, the free sources listed in the card, and the repo.
- They must not open the research repo, Winamp source, SDK material or disassembly, even briefly.
- A PR states that this was followed, using the provenance checklist in `.github/pull_request_template.md`.
- If an implementer needs a fact that no spec contains, it files a spec request. It does not look for the answer elsewhere.

## 6. Precedence rule

The alexfreud repository is Winamp **5.x**. Rustamp targets **2.95**. Therefore:

- Every behavioral spec must be validated against the real 2.95 binary, using automation, screenshots, timing or `out_disk` captures.
- Where a 5.x source and the 2.95 binary disagree, **2.95 behavior wins**.
- A spec that cites only 5.x sources, without 2.95 validation, cannot be marked validated.

## 7. Third-party code and attribution

Free-licensed code may be copied into Rustamp, with these requirements:

1. Keep the original copyright and license header in every copied file or function, unchanged.
2. Record the source, the license, the upstream version or commit, and the path in Rustamp in the THIRD_PARTY notice (`THIRD_PARTY.md` at the repository root).
3. Add an attribution comment at the top of the copied section if the license requires it (MIT, zlib and BSD all do).
4. Sources in this category are DeaDBeeF, webamp (MIT), butterchurn (MIT), WDL/EEL2 (zlib), libopenmpt (BSD) and projectM (LGPL, reference only; do not copy from projectM without a separate review).
5. A PR that copies third-party code must list it in the provenance checklist with the source, license and attribution added.

Rust dependencies, including transitive and target/build/dev dependencies, are
also recorded in `THIRD_PARTY.md` against `Cargo.lock`; see
[CONTRIBUTING.md](CONTRIBUTING.md#dependencies). Registry dependencies are not
copied-source ports, but their notices and license obligations still apply.

Copying from a dirty source is never allowed, even if the code is later modified. Modifying Winamp 5.x source does not make it clean.

## 8. Lead and reviewer duties

- Reviewers reject PRs with a missing or incomplete provenance checklist.
- Reviewers check that copied code carries its license header and an entry in the THIRD_PARTY notice.
- Any suspicion of a clean-room breach is raised with the integration lead immediately. The affected work is paused until it is reviewed. Do not try to fix it by reading the dirty source again.
