# Task card S04 — Menus (spec round, dirty room)

**Goal**
Produce the audited spec **S04 (menus)** for the classic main window and the
other classic windows: the main menu and its submenus (what each item is, its
command id, its accelerator, its check/disabled state, separators), the playlist
window's menus, the equalizer window's presets menu, and the two context menus
(the main window's long-title right-click menu and the clutterbar's), plus the
window-menu button's popup. S04 is the missing spec behind several already-built
behaviours: RFC 0005's `OpenMainMenu` handling, the clutterbar's O/I/V actions,
and the window-menu button.

**Lane** S — spec (dirty room).

**Owned paths**
- `D:\code\rustamp-research\` — drafts, captures, manifests, audits.
- The exported spec `D:\code\Rustamp\specs\S04-menus.md` — **only after a PASS**
  from the Spec Auditor; export is the orchestrator's gate, so write the draft
  first and ask for the audit.

**Allowed sources**
- The reference binary in the sandbox (`winamp295`), driven through the
  capture harness — **inside the Windows Sandbox, never on the host desktop**.
- `D:\code\Rustamp\specs\S01-window-geometry.md`, `S02-main-window-behaviour.md`,
  `S03-shortcuts.md` (audited) for the geometry and the command ids already known.
- `D:\code\rustamp-research\tools\vm-capture\README.md` — the harness, the job
  schema, the 1:1 gate, `depends_on`, `expect_change`, `baseline_path`.

**Forbidden sources**
- Winamp 5.x source, SDK headers or documentation, disassembly, anything from
  `extracted/` (do not descend into it — a previous run accidentally printed file
  names from the Winamp source tree there). No downloads.

**Scope**
- **In scope:** the menu trees with their ids and accelerators; check and
  disabled states as they are observable; the menu's own geometry (widths, item
  heights, separators, the drawn font); the context menus and when each appears;
  which menu item is the "no file loaded" state; the playlist and equalizer menus
  as far as they are observable.
- **Out of scope:** implementing anything; changing S01/S02/S03; the preferences
  dialog's own pages (S04 owns the menu that opens it, not the dialog).
- **Method:** propose a capture plan first (which menu, which state, which
  reference), then author a harness **job file** for the orchestrator to launch —
  do not start a sandbox yourself. Every claim needs a capture, a manifest row
  and a stated method; a menu that cannot be measured stays open with the reason.

**Acceptance**
- [ ] A draft spec in `drafts/` in the spec house style (prose and tables, no
  code, no transliterated pseudo-code) with a provenance header.
- [ ] A job file under `tools/vm-capture/jobs/` that the orchestrator can run.
- [ ] A manifest row for every capture used.
- [ ] A Spec Auditor pass before export.

**Report back**
The draft path; the capture plan and the job path; what is settled, what is
provisional, what stays open; every live interaction with the method recorded.
