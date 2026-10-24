# Task card M9 — Jump-to-time with a playing track (job only)

**Goal** The dialogs-geometry run measured a negative: with a stopped,
dead-entry list the Jump-to-time dialog does not open. The positive is open:
with a **playing track**, posting Ctrl+J (40193) must open it. Real media now
exists in the guest (`C:\rustamp\testmedia\*.mp3`, staged by the capture
runner), so author the job that measures it.

**Owned paths** one new job file,
`D:\code\rustamp-research\tools\vm-capture\jobs\jumptime-playing.json`, plus (if
needed) a short playlist fixture beside it. Nothing else.

**Method** load a guest media path (e.g. `C:\rustamp\testmedia\jingle-rockdown.mp3`;
35 KB, plays in seconds) via the playlist or a command the shortcuts spec binds,
wait until the transport reads playing (the position bar moves; `IPC_ISPLAYING`
answers 1), then post `WM_COMMAND` 40193 and capture the dialog
(`expect_client_size: "none"`, `depends_on` the open step). A second group posts
it while paused, to separate "needs playing" from "needs loaded".

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
files uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Remember `expect_change` asserts that something changed, never
*what* changed. Validate with `-ValidateOnly` in a temp fixture if you can.

**Report** the job path and step count; the exact command line; what each group
settles; what it cannot settle; confirmation that you started no sandbox.
