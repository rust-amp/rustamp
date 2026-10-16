# Task card S10b — The remaining S10 batches (jobs only)

**Goal** The S10 IPC spec has 58 of 69 steps measured but is not audit-ready: the
EQ-window batch, the posted-vs-sent batch and the destructive batch are missing,
and one anomaly (id 800 returning a pointer) is unexplained. Author the **job
files** for those batches. No capture run — the orchestrator launches.

**Owned paths** new job files only, under
`D:\code\rustamp-research\tools\vm-capture\jobs\` (`s10b-eq-window.json`,
`s10b-posted-vs-sent.json`, `s10b-destructive.json`). Do not touch
`drafts\S10-ipc.md`, `jobs\s10-ipc.json` (another lane owns them) or any harness
file.

**Allowed sources** `tools\vm-capture\README.md` — in particular the
**`send-message` primitive** (`msg`, signed 32-bit `wparam`/`lparam`, optional
`expect_reply`, a synchronous send when a reply is expected) and its trap: a
reply is recorded as a **zero-extended unsigned 32-bit** string, so **a step
cannot assert a negative reply** — write negatives in unsigned form (4294967295
for −1). Also `jobs\s10-ipc.json` (read-only) and the run records in `_runs\`.

**Scope, per batch**
1. **EQ window**: the `127`/`128` effects are invisible in main-window captures,
   so the batch must target the `Winamp EQ` window (`expect_client_size: "none"`,
   its own size) and write a band value ≠ 31 so the stateful write is conclusive.
2. **Posted vs sent**: post the query ids (no reply expected) and compare the
   *state* effect against the sent variants; then post `0x0402` with a loaded
   playlist to test track advance (S09 U-1).
3. **Destructive**: `IPC_RESTARTWINAMP` (135) and the playlist ids (101/121/102/
   106) with a loaded list — each in its **own launch**, since they end the run's
   usefulness.
4. **The id-800 anomaly**: probe it again in a fresh launch and record whether the
   pointer value is stable; if it is, say so in the job's `_comment` so the
   auditor sees it is reproducible.

**Hard rules** no sandbox start, no live input, no downloads, no git writes, leave
files uncommitted, touch nothing under `D:\code\Rustamp`, never descend into
`extracted/`. Validate each job with the harness's `-ValidateOnly` path in a temp
fixture if you can.

**Report** each job's path and step count; the exact command line per job; what
each settles; which S10 items remain unmeasurable without a plugin DLL;
confirmation that you started no sandbox.
