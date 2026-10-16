# Task card M7 — Reconstruct the missing measurement memo (O5/O6/O10/O15)

**Goal** The memo the RFC 0005 measurements were supposed to be recorded in —
`notes\2026-10-15-wm-command-o5-o6-o7-o10-o15.md` — **does not exist** in either
tree, so the written evidence for "the volume-key step is 4" (on which RFC 0005
§17 and the shipped `VOLUME_STEP` rest) is missing. Reconstruct it and mark it
honestly as a reconstruction.

**Owned paths** one new file,
`D:\code\rustamp-research\notes\2026-10-16-wm-command-o5-o6-o7-o10-o15-reconstructed.md`.
Nothing else.

**Allowed sources** the *recorded* results, which do exist: `docs/rfc/0005-app-commands.md`
§§7–8 and §17 (the values and the decisions they drove), `docs/HANDOFF.md` §5
(the item that records the measurements and their status), and any capture or
manifest in `_runs\` that belongs to that measurement session (look for the run
directories and the WndCommand probe steps). The earlier agent's *report* is not
in the tree — do not invent it.

**Scope** the memo records, per item: **O5** (do the playlist add commands
1032/1036/1039 play — measured as a no-op on the main window, still open because
the playlist-window ids need a dialog confirm), **O6** (40191 opens the
Preferences dialog at the Plug-ins > Visualization page), **O7** (40317 opens the
same dialog at the Options > Visualization page), **O10** (the volume-key step is
**4**, saturating at 0/255 with partial end steps), **O15** (end-of-list has no
recorded public id; needs a message spy or the real gesture). For each: the value,
the method (posted `WM_COMMAND`/`WM_USER` messages with the window proven by
`WindowFromPoint`, no synthetic input, DPI awareness, coordinate space), and the
provenance. **The file must say in its header that it is a reconstruction written
after the fact from the RFC and the handoff**, and must not claim captures that
cannot be pointed at.

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
the file uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Prose and tables only.

**Report** the file path; per item: the value, whether it is backed by something
in the tree or only by the RFC/handoff; anything you could not back; confirmation
that you started no sandbox.
