# Task card M8 — Re-measure O5/O6/O7/O10/O15 and make captures (job only)

**Goal** M7 reconstructed the missing memo, and its honest result is that **none**
of the five measurements is backed by anything locatable: no capture, no manifest
row, no job, in any of the 21 run directories. The shipped `VOLUME_STEP = 4`
(RFC 0005 §17, criterion 6) therefore rests on an unbacked claim. Author the
**job** that re-measures them and produces the evidence.

**Owned paths** one new job file,
`D:\code\rustamp-research\tools\vm-capture\jobs\o5-o6-o7-o10-o15.json`, plus (if
needed) a short playlist fixture beside it. Nothing else; other lanes own the
`*10b*`, `held-states` and `s01r2-*` jobs, and the harness belongs to another lane
still.

**Allowed sources** the reconstruction
`notes\2026-10-16-wm-command-o5-o6-o7-o10-o15-reconstructed.md` (read it: it names
the value, the method and the provenance per item), `tools\vm-capture\README.md`
(the job schema, `expect_change`, `restart`, the **`send-message` primitive** —
`msg`, signed 32-bit `wparam`/`lparam`, optional `expect_reply`, and the trap that
a reply is a zero-extended unsigned 32-bit string), and `jobs\s10-ipc.json` as a
worked example of posted `WM_USER` probes.

**Measure, with a capture for each** — this is the point of the job; a step that
produces no manifest row measures nothing:
1. **O10 (the important one)** — the volume-key step. Drive the volume up and down
   with the keyboard binding and read `IPC_GETVOLUME` (`WM_USER`, id 122) after
   each press: the step must be **4** per key event, saturating at 0 and 255, with
   the partial end steps (253→255 and 2→0) recorded. This is the measurement that
   puts the shipped constant on evidence.
2. **O6/O7** — post `WM_COMMAND` 40191 and 40317 and capture the dialog each
   opens (`expect_client_size: "none"`), so the "same dialog, different page"
   claim has a picture each; if the page is not distinguishable by pixels, say so
   in the job's `_comment` rather than implying it is.
3. **O5** — the playlist add commands 1032/1036/1039 posted to the **main** window
   (the no-op claim) and to the **playlist** window, with a state capture each.
4. **O15** — end-of-list: post `IPC_GETLISTPOS`/`IPC_GETLISTLENGTH` and the
   transport queries at the end of a short list, and record whether any id moves
   when the list ends; if the id cannot be found this way, say so plainly and name
   what would (a message spy or the real gesture).

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
files uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Remember `expect_change` asserts that something changed, never
*what* changed. Validate with `-ValidateOnly` in a temp fixture if you can.

**Report** the job path and step count; the exact command line; per item what the
steps would settle and which ids they post; anything that cannot be measured with
the current primitives; confirmation that you started no sandbox.
