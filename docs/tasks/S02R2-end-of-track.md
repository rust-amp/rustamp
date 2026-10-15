# Task card S02R2 — End-of-track, stop-after-current and the S02 leftovers

**Goal** Close the S02 items the audited spec leaves open, above all **§8.5
end-of-track behaviour** — what the player does when the last track of the
playlist ends, and what the stop-after-current mode does (which our engine already
implements and RFC 0005 exposes as `ToggleStopAfterCurrent`). Plus the leftovers:
the volume/balance slider laws as S02 sees them (S07 is measuring them from the
equalizer side; do not duplicate that work — cross-reference), the bitrate/
sample-rate area, and the marquee's behaviour at the end of a track.

**Lane** S — spec (dirty room). **Owned paths** `D:\code\rustamp-research\` —
your draft `drafts\S02-round2.md`, your job `tools\vm-capture\jobs\s02r2-end-of-track.json`,
your captures. **Write only your own files**; sibling lanes work in the same tree.

**Allowed sources** the reference binary in the Windows Sandbox through the
capture harness (a **short playlist** played to its end is the tool here; the
harness's settle and the `expect_change` assertions let you see the transport
change); `specs/S02-main-window-behaviour.md` (audited — its §8.5 and its open
questions are your scope), `specs/S03-shortcuts.md` (the stop-after-current
binding), `docs/rfc/0005-app-commands.md` §17 (the owner's decision on
`ToggleStopAfterCurrent`) and `crates/rustamp-audio` (what our engine does today,
read-only, so you can say where the spec and our behaviour agree or differ).

**Forbidden** Winamp source/SDK/disassembly/binary, `rustamp-refbin`, and
**never descend into `extracted/`**. No downloads.

**Phase 1 (now, offline)** read the sources, write the **draft skeleton**
(one row per question: what happens at the end of the last track; whether play
stops, repeats or advances; the observable transport state; the time display; the
marquee; stop-after-current's effect and its indicator), every cell marked *to be
measured*, and author the harness **job**. A short playlist (the sandbox copy
already ships `r_o4_60.m3u`; a 2–3 entry list is better for this) played to its
end is the core of the plan; give the steps enough settle time. **Do not start a
sandbox.**

**Hard rules** prose and tables only in the draft; every claim needs a capture and
a manifest row; no git writes; leave files uncommitted; touch nothing under
`D:\code\Rustamp`.

**Report** draft path and structure; job path and step count; the exact command
line; what each step settles; whether a short playlist can be produced in the
sandbox without mapping host media (the entries' files are absent and the player
skips them — say how you handle that); confirmation that you started no sandbox.
