# Task card O1 — An OCR workflow for reading captures, and its documentation

**Goal** Several lanes now have to **read text out of screenshots** (menu labels,
dialog titles, list rows) because the capture harness can only compare pixels. One
lane did it ad hoc with `Windows.Media.Ocr` and a pixel mask. Turn that into a
**documented, reproducible workflow** that every lane can use, and choose the tool
with evidence rather than by preference.

**Candidates to evaluate**
- **The built-in Windows OCR API** (`Windows.Media.Ocr`, WinRT) — already used
  successfully in this project, no installation, works from PowerShell or C#.
- **PowerToys PowerOCR** — <https://github.com/microsoft/PowerToys/tree/main/src/modules/PowerOCR>
- **Text-Grab** — <https://github.com/TheJoeFin/Text-Grab>
- **MiniSnip** — <https://github.com/deminimis/MiniSnip>
- Anything already installed on this machine that does the job (check first).

**Owned paths** `D:\code\Rustamp\docs\notes\ocr-for-captures.md` (new — the
workflow document). If a small helper script is genuinely part of the workflow,
put it in the same note as an inline snippet **and** state where a copy would
live; do not scatter files.

**What the workflow must cover**
1. **Choosing the tool**, with the evidence: which candidates need an install,
   which are already present, which work offline, what each needs (a language
   pack?), and **what actually worked on a real capture from this project**.
2. **The exact commands**, for at least the recommended tool and the built-in
   fallback, so a lane can copy-paste them. Include how to point it at a file in
   `rustamp-research\tools\vm-capture\_runs\...\captures\`.
3. **Crop and magnify** before OCR (the capture rows are small): the mask/crop
   approach one lane used, generalised, with the numbers it needs (row height,
   padding, the scale factor that made text legible).
4. **How to report the result**: verbatim text, and an explicit **uncertain**
   marker — the project rule is that a **guessed label is worse than an
   unreadable one**, so the workflow must say how to mark doubt.
5. **A worked example** on a real capture (name the file and the exact text you
   extracted), and a second example where the text is **too small or ambiguous**,
   showing how the workflow reports that rather than guessing.
6. **Limits**: what OCR cannot do here (a menu glyph, a check mark's position, a
   colour), so a lane knows when a pixel measurement is the right tool instead.

**Hard rules**
- **Install nothing and download nothing** — no `winget`, no MSI, no portable
  download. If a candidate would be the best choice but needs an install, say so
  and recommend it **to the owner as a question**, do not install it.
- Never run a reference binary, never start a sandbox, never read
  `rustamp-research\extracted\` or `rustamp-refbin`.
- No git write commands; leave the file uncommitted.
- You may **fetch the three repository pages** (README/usage) to describe what
  each tool is. If the web is unreachable, say so and rely on the built-in API
  plus what is installed locally.
- English.

**Report** the document path; the tool comparison with the evidence; the exact
commands that worked on a real capture; the worked example with the extracted
text; anything the owner would have to approve; and confirmation that you
installed nothing, downloaded nothing and started no sandbox.
