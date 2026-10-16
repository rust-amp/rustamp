# Task card S02R3 — Natural end-of-track with real media (job only)

**Goal** S02 round 2 settled the end-of-**list** state on dead entries, but the
**natural end of a playing track** was never measured (the harness could not
play anything). Real media now exists in the guest
(`C:\rustamp\testmedia\*.mp3`, staged by the capture runner), so author the job
that measures it.

**Owned paths** one new job file,
`D:\code\rustamp-research\tools\vm-capture\jobs\s02r3-natural-end.json`, plus (if
needed) a short playlist fixture beside it. Nothing else.

**Method** load a short guest file (e.g. `C:\rustamp\testmedia\jingle-debilintro.mp3`;
80 KB, seconds long), play it, and capture the transport as it ends: the
indicator, the time display, the marquee and the position bar at and just after
the end, plus `IPC_ISPLAYING` and the list position. Give the steps enough
settle time; a second group repeats it with repeat armed, a third with two
entries to see the advance. State in `_comment` what each group settles.

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
files uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Validate with `-ValidateOnly` in a temp fixture if you can.

**Report** the job path and step count; the exact command line; what each group
settles; what it cannot settle; confirmation that you started no sandbox.
