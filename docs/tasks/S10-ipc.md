# Task card S10 — The classic IPC surface (`WM_WA_IPC`), spec round phase 1

**Goal** Produce the audited spec **S10**: the classic IPC surface a Winamp 2.x
plugin uses — which `WM_USER`-range message ids exist, what each expects in
`wParam`/`lParam`, what it returns, which of them a plugin host must emulate, and
which of them our own engine must answer. S10 is the spec **M3 (plugin host)**
cannot start without, and the notes contain only a first seed list.

**Lane** S — spec (dirty room). **Owned paths** `D:\code\rustamp-research\` —
your draft `drafts\S10-ipc.md`, your job `tools\vm-capture\jobs\s10-ipc.json`,
your captures. **Write only your own files**; other lanes work in the same tree.

**Allowed sources** the reference binary in the Windows Sandbox through the
capture harness — note that the harness's **posted-message** path is exactly the
tool for this: post a `WM_USER` message to the reference and read the reply and
the observable state. `docs/notes/` in the repo if a seed list exists
(`notes/S10-seed.md` lives in the research repo), `specs/S09-plugin-abi.md`
(audited) for what the ABI already fixes, `crates/rustamp-plugin-abi/**` and
`crates/rustamp-ipc/src/message.rs` for the ids our side already speaks, and the
measurement memo from the RFC 0005 work (`notes\2026-10-15-wm-command-o5-o6-o7-o10-o15.md`)
which already proved the method works (`WM_USER` with a negative `wParam`).

**Forbidden** Winamp source/SDK/disassembly/binary, `rustamp-refbin`, and
**never descend into `extracted/`**. No downloads.

**Phase 1 (now, offline)** read the sources, write the **draft skeleton** with
one row per candidate id (id, direction, arguments, return, emulated-by-host /
answered-by-player / unknown), every cell marked *to be measured* where you have
no evidence, and author the harness **job** that measures a first batch by
posting ids and reading replies. **Do not start a sandbox.** Phase 2 after the
orchestrator's run.

**Out of scope** implementing anything; the plugin call order and lifetime (S09);
embed windows (S11); anything that needs a real plugin DLL loaded.

**Hard rules** prose and tables only in the draft, no code and no transliterated
pseudo-code; every claim needs a capture and a manifest row; no git writes; leave
files uncommitted; touch nothing under `D:\code\Rustamp`.

**Report** draft path and structure; the id list you plan to measure; job path
and step count; the exact command line; what each batch settles; the ids you
cannot measure without a plugin DLL; confirmation that you started no sandbox.
