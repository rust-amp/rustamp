# Task card S06 — The playlist window's own behaviour, spec round phase 1

**Goal** Produce the audited spec **S06**: what the playlist editor window does on
its own — its buttons (add, remove, select, misc, list opts), the sort/randomise
menu, the list interactions (single click, double click, drag within the list,
keyboard), the list's own scrolling and the title bar — as far as the harness can
measure it. S04 covers the *menus* and S01 the *geometry*; nothing yet covers the
playlist's behaviour.

**Lane** S — spec (dirty room). **Owned paths** your own lane directory:
`D:\code\rustamp-lanes\s06\` with `drafts\S06-playlist.md`,
`jobs\s06-playlist.json` and your captures. The orchestrator copies the files into
the canonical tree and launches the run. **Never write into
`D:\code\rustamp-research` or `D:\code\rustamp`** (read them freely). Never descend
into `extracted\`; never read `D:\code\rustamp-refbin`.

**Allowed sources** the reference binary in the Windows Sandbox through the
capture harness — read `tools\vm-capture\README.md` first: the job schema, the
**`send-message`** primitive (a synchronous send with a timeout when a reply is
expected; a reply is a **zero-extended unsigned 32-bit** string), the
**`may_change_client_size`** flag for an action that resizes the window, and
**`expect_same: "<step>"`** for identity to a named step (`expect_change` asserts
only *that* something changed, never *what*). Then `specs/S01-window-geometry.md`
and `specs/S04-*` if a draft exists (read-only) for what is already known, and
`specs/S02-main-window-behaviour.md` for the house style.

**Forbidden** Winamp source, SDK headers, disassembly, the binary outside the
harness, `rustamp-refbin`, and **never descend into `extracted/`**. No downloads.

**Phase 1 (now, offline)** read the sources, write the **draft skeleton** — one
row per question, every cell marked *to be measured* where you have no evidence —
and author the harness **job**. Cover at least: the five bottom-bar buttons of the
playlist window and what each opens or does; the **sort/randomise** menu's entries
and their effect on a loaded list; what a single click, a double click and a drag
inside the list do; the list's scrolling; whether the list's own title bar has a
menu button (the S04 run found none on the main window's neighbours — check);
and what the window's `Winamp PE` class exposes. Use the **playlist window**
(`Winamp PE`) as the target where the action belongs to it, and the main window
for messages (a sibling lane measured that the **EQ window does not forward
`WM_USER`**; assume the playlist window may behave the same and design the job so
a refusal is visible rather than silent). **Do not start a sandbox.**

**Hard rules** prose and tables only in the draft; every claim needs a capture and
a manifest row; no git writes; leave files uncommitted; never write outside your
lane directory.

**Report** the draft path and structure; the question list you plan to measure;
the job path and step count; the exact command line; what each group settles;
what you cannot measure with the current primitives; confirmation that you started
no sandbox and wrote only inside your lane directory.
