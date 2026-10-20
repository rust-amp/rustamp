# RFC 0021: the orchestrator session may act as the dirty room for spec authoring

Status: **Draft** (2026-10-20). Drafted at the owner's direction; the owner
accepts it by using the work it produces. Nothing here changes a crate.

## 1. What changes

`CLEANROOM.md` §2 gives the dirty room to "R-agents" in the private research
repository and keeps the orchestrator clean:

> **Orchestrator and integration lead.** These roles stay clean too. … they
> must not read dirty sources to resolve an implementation question.

The owner has decided (2026-10-20) that the orchestrator session may take the
**dirty-room role** for spec authoring: it may run the reference binary and may
disassemble it, **provided** that

1. the only artifact it produces from that material is a **spec draft**;
2. the draft is audited (§3) before anything reaches `specs/`;
3. **no code is written from the dirty material** — implementation is done by a
   different agent, working only from the audited spec (the owner's words:
   *"aus dem disassembly wird nur spec erzeugt, aus der spec erzeugt ein anderer
   agent neuen code"*).

The separation that §2 protects is therefore **role**-based, not
session-based: the session that reads the binary never writes the code that
implements it.

## 2. What does **not** change

- §4's content rules are unchanged and are what make the export safe: a spec
  may state behaviour, numbers, layouts, constants and ABI facts, and **must
  not** contain source code, transliterated pseudo-code, copied comments,
  distinctive internal identifiers or disassembly listings.
- §3's pipeline is unchanged: draft → auditor → `specs/`, with the audit date in
  the provenance header. A file without an audit date still does not belong in
  `specs/`.
- §5 still binds **implementers**: the agents that write code receive the task
  card and the audited spec, and must not open the binary, its disassembly, the
  research repository or SDK material.
- §6 still binds: 2.95 wins over 5.x, and a behavioural spec is validated
  against 2.95.

## 3. The new artifact: drafts live outside the repo

A dirty-room draft is **not** committed to `rustamp`. It is written beside the
reference material, at `D:\code\rustamp-sandbox\spec-drafts\<id>.draft.md`,
which is outside the repository and outside the research repository.

The auditor is a **separate agent** that reads the draft and the repo only, and
writes the audited spec into `specs/`. Drafts never enter the repository, so an
implementer cannot reach one by browsing the tree.

## 4. Why this is not the breach §8 warns about

§8 says a suspected breach pauses the work and is reviewed rather than repaired
by reading more dirty material. This RFC does not repair a breach; it records an
**owner decision that widens one role** and keeps every content and
compilation-time safeguard in place. The decisive property is unchanged:
**the person who reads the binary and the agent who writes the code are
different, and the only thing crossing the boundary is an audited prose spec.**

Two further safeguards:

- Every spec produced under this RFC names its source **category** in the
  provenance header, so a later reviewer can see at a glance that the evidence
  is disassembly-derived.
- The draft file is kept, so the auditor's changes remain reviewable.

## 5. What the owner is accepting

1. The clean-room defence now rests on **role separation plus §4 auditing**
   instead of on physical separation of the reading session. A future reviewer
   must be able to see, from the audit note alone, that no code crossed.
2. The auditor is the only gate. A rushed audit is now the main risk, so the
   audit note must say what was removed, not only that the spec was approved.

## 6. Acceptance

- `CLEANROOM.md` §2 gains one sentence: *"At the owner's direction (RFC 0021) a
  session may take the dirty-room role for a single spec, provided the spec is
  audited and no code is written from the dirty material in that session."*
- This RFC is referenced from the provenance header of every spec produced
  under it.
- No crate changes.
