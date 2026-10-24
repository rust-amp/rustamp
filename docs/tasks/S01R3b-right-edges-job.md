# Task card S01R3b — The discriminator and the remaining S01 probes (job only)

**Goal** Author the harness job that closes the S01 items the round-3 audit left
open. **No capture run** — the orchestrator launches. The audit names exactly what
is needed; this card turns that into a job file.

**Owned paths** `D:\code\rustamp-research\tools\vm-capture\jobs\s01r2-right-edges.json`
(new). Write nothing else in the research tree — other lanes work there, and the
existing job `s01r2-open.json` is **frozen** (it is the export basis; do not touch
it).

**Allowed sources** `tools\vm-capture\README.md` (the job schema, the 1:1 gate,
`landmark_region`, `depends_on`, `expect_change`, `baseline_path`, the settle),
`jobs\s01r2-open.json` as the worked example, `audits\S01-round3-audit.md` (the
open items and the capture each needs), `audits\S01-round3-recheck.md` (the
numbers now independently reproduced), and the repo spec
`D:\code\Rustamp\specs\S01-window-geometry.md` §7 (read-only) for the item list.

**The job must implement, per the audit:**
1. **The discriminator**, one group each for REM at (46,92) and SEL at (74,92):
   fresh launch → settle → posted select at (60,28) → **one** posted click at the
   point → capture → **a second capture ~3 s later** with `input_method: none`.
   The first capture showing the change means the click acted; only the second
   means the grid's change was timed, and then **neither** edge is measured.
2. **Re-contextualised right-edge probes** — in the grid's context (either the
   same preceding gap clicks at x=36–44 or ≥15 s after the selection), one launch
   per candidate: REM x = 47, 49, 51, 53, 55; SEL x = 75, 77, 79, 81, 83.
3. **Real clicks at y=100** for REM, SEL and LIST OPTS (`input_api: mouse_event`,
   two clicks each, then the post-click state capture — REM/SEL open no dialog, so
   a `#32770` capture cannot exist; ADD/MISC already have their y=100 groups).
4. **The `WinampVis` right edge** with **no** MISC click open: a 1-px grid at
   x = 122…126, reading the hit class from each pre-flight line.
Every step: `expect_client_size` where the window is 275×116, `expect_change`
where a click must have an effect, `depends_on` where a popup is involved.

**Hard rules** no sandbox start, no live input, no downloads, no git writes,
leave the file uncommitted, touch nothing under `D:\code\Rustamp`, never descend
into `extracted/`. Validate the job with the harness's own `-ValidateOnly` path
against a temp fixture if you can do it without launching a sandbox; otherwise
state that it is unvalidated.

**Report** the job path and step count; the exact command line; what each group
settles; which of the audit's open items it closes and which it cannot; whether
you could validate it; confirmation that you started no sandbox.
