# Task card H1 — Held states with a lone button-down (job only)

**Goal** The harness releases the mouse button in a `finally` **before** it
captures, so a **held** state cannot be captured with real mouse — which is why
the "held sprite" differences (534/300/401 px) are post-release state changes and
the four held rects of the equalizer are unmeasured. The `send-message` primitive
can post a **lone `WM_LBUTTONDOWN` (0x201) with no `WM_LBUTTONUP`**, leaving the
control held while the capture runs. Author the job that measures the held states
that way.

**Owned paths** one new job file,
`D:\code\rustamp-research\tools\vm-capture\jobs\held-states.json`. Nothing else —
another lane owns `s10b-*`, and the frozen `s01r2-open.json` must not be touched.

**Allowed sources** `tools\vm-capture\README.md` (the job schema and the
`send-message` primitive: `msg`, signed 32-bit `wparam`/`lparam`, optional
`expect_reply`; a reply is a zero-extended unsigned 32-bit string), the S01
round-2 and S07 evidence in `drafts\`/`audits\` for the rects in question, and
`D:\code\Rustamp\specs\S01-window-geometry.md` §3.1/§4.1 (read-only) for what is
claimed today.

**Scope** measure, each in its own launch so no held state leaks into the next
step:
1. the equalizer's **On** and **Auto** held rects, engaged and not engaged (the
   S01 round-2 rects came from *posted* messages; re-establish them with the same
   posted-down technique and compare against the recorded captures);
2. the **Presets** button held (the 534-px case) — is the difference a real held
   sprite or a post-release change?
3. the **volume** and **balance** slider held at several positions, reading the
   `VOLUME: n%` / `BALANCE: …` hint that only appears while the button is down —
   this is the capture the S07 lane named as the one that would settle both laws;
4. the dragged **preamp thumb** mid-drag, to compare with the round-2 result.
Each: post the down at the control's point, capture, then post the up in a
separate step (so the release is recorded too). `expect_change` where a change
must happen; `expect_client_size` 275×116 on the main/EQ windows and `"none"`
where a popup or a different window is the target.

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
the file uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Validate with `-ValidateOnly` in a temp fixture if you can.

**Report** the job path and step count; the exact command line; which claims each
group would settle; anything you cannot express with the available primitives;
confirmation that you started no sandbox.
