# Spec request: what a Winamp 5.x plug-in expects of its host (S21)

Filed 2026-10-25 by the orchestrator. **A request to the dirty room, not work.**
The orchestrator and the implementers stay clean ([CLEANROOM.md](../../CLEANROOM.md)
§2, §5): nothing below may be built until the dirty room has described the
behaviour in its own words, an auditor has passed it and it is exported to
`specs/`.

## Why

The host accepts 5.x plug-in headers since 2026-10-25 (owner decision; S09 U-7
closes as a range). Of fourteen real Winamp 5.66 plug-ins in the Hyper-V VM,
three load (`gen_hotkeys`, `gen_jumpex`, `out_wave`) and the rest **fault in
their own start-up**. The host answers what the audited S10 settled for the 2.95
reference: `IPC_GET_RANDFUNC` (3015) and `IPC_GET_API_SERVICE` (3025) answer
**1 = unsupported**, and the 5.x-only requests answer 0 or 1. A 5.x plug-in
written for a 5.x host takes some of those answers as pointers.

The runtime side is solved and recorded in
`docs/notes/real-plugin-survey.md` (a modal C-runtime dialog, the missing
side-by-side assembly). What remains is behavioural.

## What was observed (our own host, the plug-ins run only in the VM)

`RUSTAMP_HOST_TRACE` writes every non-system window message a plug-in sends the
host window; the fault report names the instruction and the address touched.
Plug-in, the call it faulted in, what it sent the host first, and what it
touched:

| plug-in | faulted in | sent to the host window first | touched |
|---|---|---|---|
| `out_disk` | `init` | `WM_USER` 334, then 3025 | read 0x0 |
| `in_vorbis` | `init` | `WM_USER` 3015 (wparam 0, then 1) | read 0x0 |
| `dsp_sps` | `init` | `WM_USER` 336 | read 0x1 (inside a system DLL: a handle or pointer of value 1 was used) |
| `in_flac`, `in_wave` | `init` | nothing | read 0x0 |
| `vis_avs`, `vis_milk2` | the header getter | nothing | read 0x0 |
| `gen_hotkeys`, `gen_jumpex`, `out_wave` | loaded fine | nothing | - |

(`vis_nsfs` returned no module at all: its getter answers null.)

## Questions the dirty room can settle

1. For each of the seven faulting plug-ins: **what does it read through a null
   (or 1) value, and where does that value come from?** Candidates, to be
   decided by observation: the answer to an IPC request, a field of the module
   struct the host should have filled, an export of the host executable, a
   service the plug-in looks up through the answer to 3025, a window it
   expects (a child, a property).
2. What does `IPC_GET_API_SERVICE` (3025) return to a plug-in on a 5.x host, and
   which of the services does each of these plug-ins look up? Describe each
   used service's **observable contract** (what the plug-in asks, what it needs
   back), not the headers' text. In particular: the memory manager, the
   application service, the configuration service.
3. What do ids **334**, **336** and 3015 mean (S10 §13.3 left 334/335/336 "never
   probed" and 3015's pointer unobserved), what shape do the 5.x plug-ins need
   as the answer, and what does a plug-in do with it?
4. Which fields of the input, output, DSP and visualization module structs does
   a 5.x host fill that a 2.x host does not (S09 marks several `[5X-ONLY]`)?
5. What does a 5.x host do with a plug-in that finds **no** service manager:
   does `in_wave`/`in_flac` still start (it must, on a 5.x host that has none
   for them), and what does it print or show?

## Boundary

* The 2.95 answers in S10 stay as they are (measured on the reference). A host
  that serves 5.x services is an **extension** and must not change what a
  plug-in sees when it asks a 2.95 question; the spec should say how the host
  decides which one it is talking to (probably: only for plug-ins whose header
  version is 5.x).
* Services whose implementation would mean redistributing Winamp 5 code (the
  media library, the skin engine, `gen_ff`) are out of scope: say which services
  a plug-in needs, and the clean room decides what to provide.

## Next step in the tree

Nothing here is implemented. The wire the observations came from is the survey
test (`crates/rustamp-plughost/tests/real_plugin_survey.rs`, run with
`docs/notes/vm-run-gui-session.ps1 -Filter real_plugin_survey`); the plug-ins
are in `D:\code\rustamp-testplugins\w566` (extracted from the 5.66 installer,
not in the repository).
