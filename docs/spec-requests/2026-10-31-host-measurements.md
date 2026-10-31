# Spec request: audit the rendering laws measured on the host (2026-10-31)

Filed 2026-10-31. **A provenance record and a request for an audit.**

## What happened

On 2026-10-31 the owner allowed Winamp 2.95 to run on this machine, for a pixel
comparison with Rustamp, and forbade disassembling it. The session that compared
them also fixed the differences, from what it measured. That is black-box
measurement of the running windows only: no disassembly, no source, no SDK.

It is still **not the path of [CLEANROOM.md](../../CLEANROOM.md) §2-§3**, where a
dirty-room session measures, an auditor checks and a different session implements
from the exported spec. The rule that a reference binary runs only in the VM
(owner approval of 2026-10-17, restated in `docs/HANDOFF.md` §1) was also set aside
for this comparison, by the owner.

The owner should decide whether this exception stands. Until then, these are
measurements without an audit.

## Where it landed

Commit `9c5314d` and the commits before it on `fix/gui-host-test-findings`. The
code comments say "measured against Winamp 2.95, 2026-10-31" at each law:

- the equalizer: thumb position, 28 band sprites, curve interpolation;
- the main window: volume thumb and key step, percent rounding, the position bar;
- the playlist: row bands, length margin, status fields;
- the shaded main window: analyser place and floor, time cells, three thumbs;
- the seek and first-entry behaviour of the engine and the app.

## What is asked

Re-measure these in the VM, audit them and export them into S01, S02, S06 and S07,
so that the code's numbers have the provenance every other number has.
