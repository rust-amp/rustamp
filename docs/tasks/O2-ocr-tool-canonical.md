# Task card O2 — A canonical OCR tool: source in the repo, binary outside it

**Goal** The OCR workflow (`docs/notes/ocr-for-captures.md`) currently makes every
lane rebuild a small C# helper from the source inlined in the note. Give it a
canonical shape, per the owner's decision: **the source and a build script live in
the repo; the built binary lives outside it** in `D:\code\tools\ocr\`. Text-Grab is
now installed, so its file CLI joins the workflow too.

**Lane** tooling. **Owned paths** a new worktree; inside it, `tools/ocr/**` (the
source + the build script, plus a short README). Outside the repo you may write
only under **`D:\code\tools\ocr\`** (the build output). Do not touch the crates,
`xtask`, `specs/**` or the capture harness — report those needs instead.

**Allowed sources** `D:\code\Rustamp\docs\notes\ocr-for-captures.md` (the workflow
you are formalising — its §A holds the helper source that already worked),
`docs/tasks/O1-ocr-workflow.md` (the card behind it), the repo's `CONTRIBUTING.md`
and `AGENTS.md` for the conventions. **Clean room:** never read Winamp source, SDK
headers, disassembly, the 2.95 binary, `rustamp-research` or `rustamp-refbin`.

**Do**
1. Put the helper's **C# source** and a **build script** (PowerShell) under
   `tools/ocr/` in the repo, with a short README saying what it is, why it exists,
   and how to build it. Keep the source ours and say so.
2. The build script builds to **`D:\code\tools\ocr\`** (create it if missing) and
   takes the output path as a parameter with that as the default; it must fail
   loudly if the Windows SDK facade it needs is missing.
3. **Run the build** and confirm the binary lands in `D:\code\tools\ocr\`.
4. **Verify both tools on a real capture** and record the output: the built
   `Ocr.exe` on one capture, and **Text-Grab's file CLI** (it is installed now —
   find its `--windowless` usage and its exact flags) on the same capture. Say
   which is easier and for what, and update the recommendation only if the
   evidence supports it.
5. **Update `docs/notes/ocr-for-captures.md`**: the canonical binary path, the
   build command, the Text-Grab commands, and a note that the source now lives in
   the repo rather than inline. Keep the crop/magnify numbers and the
   **`[uncertain: …]`** rule exactly as they are.

**Hard rules** clean room; **install nothing further and download nothing**;
no git write commands; **run no test, build or window of the Rust workspace** (the
C# build is not a workspace build and is fine); no window may be opened — do not
launch Text-Grab's GUI; English.

**Prove it works.** Quote the built binary's path, its size, and the text both
tools extracted from the same real capture.

**Report** the files and their paths; the build command; the binary's path and
size; the two tools' output on the same capture and which you would recommend for
what; the note as updated; confirmation that you installed nothing further,
opened no window and ran no git write.
