# Task card S07 — Equalizer interior, DSP order and the value laws (spec round)

**Goal** Produce the audited spec **S07**: the equalizer's remaining interior
(the slider track decomposition and body colouring, the dragged and held
sprites, the two graph line strips, the On/Auto held states, the preset
handling), **the volume and balance laws**, the EQ band step sizes, and **the
order of EQ and DSP in the chain**. S07 is the missing spec behind the EQ window,
RFC 0005's band-step questions (O11) and the engine's provisional volume/balance
laws (`rustamp-audio/src/gain.rs`).

**Lane** S — spec (dirty room). **Owned paths** `D:\code\rustamp-research\` —
your draft, captures and job file. **Write only your own files** (`drafts\S07-*`,
`jobs\s07-*`); other lanes work in the same tree. Export to
`D:\code\Rustamp\specs\S07-*.md` only after a Spec Auditor PASS.

**Allowed sources** the reference binary in the Windows Sandbox through the
capture harness (`tools\vm-capture\README.md`); the audited
`specs\S01-window-geometry.md` (its round-2 equalizer interior and the open items
it lists), `S02-main-window-behaviour.md`; `docs\HANDOFF.md` §5 (the value-range
and EQ follow-ups) and `docs\rfc\0005-app-commands.md` §16 (O11).

**Forbidden** Winamp source/SDK/disassembly/binary, `rustamp-refbin`, and
**never descend into `extracted/`**. No downloads.

**Scope** In: the EQ track's decomposition and per-band colouring as a function
of the value; the dragged thumb and the held On/Auto rects (partly measured in
S01 round 2 — build on it, do not re-derive); the graph strips; the presets
menu/load path; the **volume law** (what the slider's position means in the
0..=255 domain) and the **balance law**; the EQ band step; and the EQ/DSP order
as far as it is observable. Out: implementing anything; S01/S02/S03 changes;
MilkDrop/AVS.

**Method** Phase 1 (now, offline): read the sources, write the draft skeleton
with every cell marked *to be measured*, and author `jobs\s07-eq-dsp.json`.
**Do not start a sandbox.** Phase 2: interpret after the orchestrator's run.

**Acceptance** draft in the house style with a provenance header; a validated job
file; a manifest row per capture; an auditor PASS before export.

**Report** draft path and structure; job path and step count; the command line;
what each step settles; open questions with no capture planned; that you started
no sandbox and touched nothing under `D:\code\Rustamp`.
