# Task card S05 — Docking, snapping, windowshade and double-size (spec round)

**Goal** Produce the audited spec **S05**: how the classic windows dock and snap
to each other and to the screen edges, the windowshade behaviour (all three
windows), double-size mode, the window positions and their persistence, and what
the app must do on a drag or a snap. S05 is the missing spec behind the UI's
`ToggleWindowShade`/`ToggleDoubleSize` commands, the docking placeholder and the
DPI/window-sizing remainder.

**Lane** S — spec (dirty room). **Owned paths** `D:\code\rustamp-research\` —
your draft, your captures, your job file. **Write only your own files**
(`drafts\S05-*`, `jobs\s05-*`); other lanes work in the same tree. Export to
`D:\code\Rustamp\specs\S05-*.md` only after a Spec Auditor PASS — write the draft
first.

**Allowed sources** the reference binary in the Windows Sandbox, driven through
the capture harness (`tools\vm-capture\README.md`: the job schema, the 1:1 gate,
`landmark_region`, `depends_on`, `expect_change`, `baseline_path`, the settle);
`specs\S01-window-geometry.md`, `S02-main-window-behaviour.md`,
`S03-shortcuts.md` (audited) for geometry and the commands already known;
`docs\HANDOFF.md` §5 for the DPI/window-sizing findings.

**Forbidden** Winamp source/SDK/disassembly/binary, `rustamp-refbin`, and
**never descend into `extracted/`**. No downloads.

**Scope** In: the snap rules (which edges, what tolerance, what happens to the
docked window when the other moves), the shade behaviour per window (which rows
are drawn, what the title bar becomes, what the buttons do), double-size (what
scales, what does not), window positions and whether they persist, and the
observable difference between "docked" and "merely adjacent". Out: implementing
anything; S01/S02/S03 changes; the preferences UI.

**Method** Phase 1 (now, offline): read the sources above, write the draft
skeleton with every cell marked *to be measured*, and author a harness **job
file** `jobs\s05-docking.json`. **Do not start a sandbox** — the orchestrator
launches. Phase 2 (after the run): interpret into the draft, mark
settled/provisional/open.

**Acceptance** draft in the house style (prose and tables, no code, no
transliterated pseudo-code) with a provenance header; a validated job file; a
manifest row for every capture used; an auditor PASS before export.

**Report** draft path and structure; job path and step count; the exact command
line; what each step settles; open questions with no capture planned; that you
started no sandbox and touched nothing under `D:\code\Rustamp`.
