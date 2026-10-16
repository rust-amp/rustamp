# OCR for captures — reproducible workflow

How every lane reads text out of `vm-capture` screenshots
(`rustamp-research\tools\vm-capture\_runs\<run>\captures\*.png`).
The capture harness compares pixels only; this document is the one
approved way to turn pixels into quoted labels.

**Recommendation up front:** use the built-in Windows OCR API
(`Windows.Media.Ocr`, WinRT) through the small `Ocr.exe` helper below.
It is already present, works offline, needs no install, and is proven on
real captures from this project (§1, §5). The canonical binary is
`D:\code\tools\ocr\Ocr.exe`, built from the repo source `tools/ocr/Ocr.cs`
via `tools/ocr/Build-Ocr.ps1` (§2a). Text-Grab is installed now and tested
(§2d): handy for a quick plain-text read, but it prints no word boxes, so
lane reports still go through `Ocr.exe`.

**Provenance of this document.** Every claim marked [RAN] was executed
on 2026-10-16 against run `20261016-081527` with the commands in §2.
Nothing was installed, downloaded, or run in a sandbox; no reference
binary was executed; `rustamp-research\extracted\` and `rustamp-refbin`
were not touched.

**Update 2026-10-16.** The helper source now lives in the repo
(`tools/ocr/Ocr.cs`, byte-identical to the former §A listing) and builds to
the canonical `D:\code\tools\ocr\Ocr.exe` (6144 bytes) [RAN]; Text-Grab
4.15.0's `--windowless` file CLI was verified on a locally rendered probe
PNG (no research capture touched, no window opened) [RAN].

---

## 1. Tool comparison with evidence

| Candidate | Install needed? | Present on this machine? | Works offline? | Language needs | File-input (scriptable)? | Verdict |
|---|---|---|---|---|---|---|
| Built-in `Windows.Media.Ocr` via `Ocr.exe` (§2) | No | Yes — OS API + Windows SDK `winmd` + .NET Framework `csc.exe` all present | Yes [RAN] | OCR language pack for the target language; `en-US` worked with no extra pack [RAN] | Yes: `Ocr.exe <png>` [RAN] | **Recommended** |
| PowerToys Text Extractor (= PowerOCR module) | No (already installed) | Yes — PowerToys `v0.101.2362.0`, `PowerToys.PowerOCR.exe` + resources present, `TextExtractor` enabled in `%LOCALAPPDATA%\Microsoft\PowerToys\settings.json`, hotkey Win+Shift+T (settings file: `win:true, shift:true, code:84`) | Yes (same WinRT engine underneath) | Same as above | **No** — interactive screen-region grab to clipboard only; no CLI that takes a PNG file | Fallback for live-screen spot checks only, not for capture files |
| Text-Grab 4.15.0 | No (already installed: `text-grab` on `PATH`, Store package `JosephFinney.Text-Grab`) | Yes | Yes (same WinRT engine; README: "All OCR runs entirely on your device") | WinRT mode: Windows language packs (`en-US` worked with no extra pack [RAN]) | Yes — `text-grab --windowless "<png>"` copies plain text to the clipboard, no window [RAN §2d] | Quick plain-text reads and spot checks; **not** a lane workflow — no stdout, no word boxes (§2d) |
| MiniSnip | **Yes** — Store or portable exe from GitHub releases (per its README) | No | Yes ("never connects to the internet") | OCR language pack, e.g. admin `Add-WindowsCapability -Online -Name Language.OCR~~~en-US~0.0.1.0` (per its README) | **No** — hotkey `Ctrl+Shift+C` snip → clipboard or `Downloads\minisnip_OCR.txt` | Same engine, not scriptable → no advantage over §2; needs a download → **ask the owner** (§7) |
| Standalone Tesseract | **Yes** | No (`where.exe tesseract` finds nothing) | Yes | `tessdata` files | Yes | Out of scope: install forbidden, and nothing in this project needs it |

Notes:

- PowerOCR, Text-Grab (default mode) and MiniSnip are all front-ends
  over the **same** `Windows.Media.Ocr` engine (Text-Grab's README says
  its Full-Screen Grab mode "is also the basis of the PowerToys Text
  Extractor"). None of them recognises anything the §2 helper cannot;
  they only differ in install cost and scriptability. That is why the
  built-in API wins: identical recognition, zero install, file-capable.
- Text-Grab `--windowless` and `Ocr.exe` were run on the same locally
  rendered probe PNG (474×81, two Arial 16 px lines) [RAN 2026-10-16]:
  both read the text correctly, but Text-Grab copies only the plain
  concatenated text to the clipboard (no stdout, no boxes) while `Ocr.exe`
  prints `LINE:`/`WORD[…]` records to stdout. Same words, different
  scriptability — which is why the recommendation stands (§2d).
- What "already present" was verified with (all read-only): `Test-Path
  "$env:ProgramFiles\PowerToys"` → True; `PowerToys.PowerOCR.exe` and
  per-locale `PowerToys.PowerOCR.resources.dll` files listed;
  TextExtractor settings read; `where.exe` misses for `tesseract`;
  `text-grab` resolves on `PATH` (Store package, version 4.15.0) [RAN
  2026-10-16]; `OcrEngine.TryCreateFromLanguage("en-US")` returned a
  working engine [RAN — every OCR run below used it].
- Prior art in this project: lane `s06r` (`rustamp-lanes\s06r\audits\S06-reading.md`)
  compiled `Ocr.cs` against `Windows.winmd` and read the MISC popup, the
  ADD/LIST-OPTS dialogs and the playlist rows with it, combined with a
  Pillow pixel mask for the blue selection block (colour `(0,0,198)`).
  This document generalises that approach (§3) and re-proves it (§5).

---

## 2. Exact commands

Work in your **own** temp directory. Never write next to the captures.

```powershell
$Run = "D:\code\rustamp-research\tools\vm-capture\_runs\20261016-081527"
$Cap = "$Run\captures"
$Work = "$env:TEMP\opencode\<your-lane>"   # e.g. ...\o1; create it, use it, keep it
New-Item -ItemType Directory -Force $Work | Out-Null
```

### 2a. Recommended: build the helper once, run it on any PNG

The helper source lives in the repo at `tools/ocr/Ocr.cs` (6 KB, prints
`TEXT_START…TEXT_END`, `LINE:` and `WORD[x,y,w,h]:` records; formerly
reproduced verbatim in §A). Build the canonical binary with the .NET
Framework compiler that ships with Windows (no SDK project, no NuGet, no
network) [RAN 2026-10-16 — exit 0, only the benign CS1701 version
warning, `Ocr.exe` 6144 bytes]:

```powershell
powershell -ExecutionPolicy Bypass -File tools\ocr\Build-Ocr.ps1
```

This writes `D:\code\tools\ocr\Ocr.exe` (created if missing; override with
`-OutputDir`). The script fails loudly if the Windows SDK facade it needs
is missing. Use the canonical binary for every lane read:

```powershell
& "D:\code\tools\ocr\Ocr.exe" "$Cap\019_pe_misc_popup.png"
& "D:\code\tools\ocr\Ocr.exe" "$Cap\007_pe_add_dialog.png"
& "D:\code\tools\ocr\Ocr.exe" "$Cap\025_pe_list_single_before.png"
```

(Output quoted in §5 [RAN].) A freshly built binary reproduces the
reference outputs byte-for-byte [RAN — §5 example 1 was re-run with a
clean-room rebuild].

### 2b. Crop + magnify, then OCR (small rows — always do this)

Playlist/list captures are 275×116 with a 13 px row pitch and ~7 px
glyphs — illegible to the engine at native size (tight 1× row crop:
**0 lines**) [RAN — §3]. Crop each row band, upscale 3×
nearest-neighbour, then run the helper (Pillow 12.3.0 verified present):

```powershell
python -c "
from PIL import Image
im = Image.open(r'$Cap\025_pe_list_single_before.png')
for i, y in enumerate([23, 36, 49, 62]):      # row-band tops for this capture family
    row = im.crop((0, max(y-2,0), 275, y+15)) # 13 px pitch + 2 px pad top/bottom
    row.resize((row.width*3, row.height*3), Image.NEAREST).save(r'$Work\row%d_3x.png' % i)
print('saved')
"
& "$Work\Ocr.exe" "$Work\row0_3x.png"
```

Row-band tops differ per capture family — locate them with the word
boxes of a full-image pass first (see §3). Always keep full width
(`x = 0..width`); never shave the left edge — the track numbers live
there and clipping the head is what breaks reads at 4× (§3).

### 2c. Built-in fallback: PowerToys Text Extractor (interactive only)

No command takes a file, so this is a spot-check tool, not a workflow.
Press **Win+Shift+T**, drag a region over the image shown on screen,
paste the clipboard result somewhere, and transcribe it by hand into
the `verbatim / [uncertain]` format of §4. Prefer §2a/§2b whenever the
file exists — which, for lanes, is always.

### 2d. Text-Grab file CLI (installed, tested — spot checks only)

Text-Grab 4.15.0 is installed (`text-grab` on `PATH`) and its file CLI was
verified on a locally rendered probe PNG [RAN 2026-10-16 — the text landed
on the clipboard, no window opened, nothing printed to stdout]:

```powershell
text-grab --windowless "D:\code\rustamp-research\tools\vm-capture\_runs\<run>\captures\<file>.png"
Get-Clipboard   # the recognised plain text is here, not on stdout
```

Use it for a quick human spot check of one file. Do **not** use it for lane
reads: it returns only the plain concatenated text (no `LINE:`/`WORD[…]`
records, so no row attribution per §3/§4), and clipboard output is fragile
in automation. Lane reports still go through §2a. Bulk folder mode
(`Text-Grab.exe "<folder>"`) opens an Edit Text window — a GUI — so
headless lanes must not use it.

---

## 3. Crop and magnify before OCR — the numbers

Generalised from the `s06r` mask/crop approach. Measured on the
275×116 playlist captures (`025`–`032` family) [RAN]:

| Parameter | Value | How it was fixed |
|---|---|---|
| Row pitch | **13 px** | Word-box tops in the full-image pass: y = 25, 38, 51, 64 (Δ = 13); matches the lane's earlier `[23,36,49,62]` bands |
| Row-band window | **pitch ± 2 px pad** → `(0, y−2, width, y+15)`, i.e. 17 px tall, full width | Pad absorbs ±1 px band error without pulling in the neighbour row |
| Glyph cap-height at 1× | **~7 px** (`WORD[…,h=6..7]` at native size) | Read off the `WORD` boxes, not guessed |
| Scale factor | **3× nearest-neighbour** → glyphs ≈ 21 px tall | Scale series on the same row crop [RAN]: 1× → 0 lines; 2× → `57. longtitle`; **3× → `57. longtitle`** (stable, bigger boxes); 4× → `onatitle` (head `57. l` lost — block edges confuse the engine). Rule of thumb: magnify until glyph height is **≥ ~20 px**, stop at 3× |
| Resampling | **`Image.NEAREST`** | The skin font has 1 px features; bilinear/bicubic blur them into grey mush. Nearest-neighbour preserves hard edges |

Two traps actually hit during this work — avoid both:

1. **Overlapping windows double-count.** A 21 px window on a 13 px
   pitch re-read row 0's text inside row 1's crop. Keep window height
   ≤ pitch + padding and confirm each row's attribution with its word
   boxes (divide scaled box coordinates by the scale factor and check
   they fall inside the intended band).
2. **One pass is not a read.** Full-image OCR of `025` gave fragments
   (`57`, `ref`, `59 ref`, `tl`) while the 3× crops gave complete rows
   (`57. longtitle`, `58. ref tone mono`, `59. ref tone stereo`) — and
   row 3's `tl` fragment did **not** reproduce in its 3× crop at all
   (§5, example 2). Confirmation rule (§4) exists because of this.

For dialogs (1503×778, e.g. `007`/`023`) no magnification is needed:
glyphs are 13–18 px natively and the full-image pass reads 31 lines
directly [RAN]. Magnify only when glyphs are under ~10 px.

---

## 4. How to report the result

A **guessed label is worse than an unreadable one.** Copy this format:

```
FILE: <run>/<capture>.png (+ crop used, e.g. "rows y=21..38 @3x")
READ (verbatim, engine output, uncorrected): "<exact LINE text>"
BOXES: <WORD[...] records that place it, scaled back to native coords>
STATUS: confirmed | uncertain
```

- **verbatim** means byte-for-byte engine output: keep `etzwerk`,
  `vunamp`, `'NAMP PLAY` exactly as printed. Never silently fix
  spelling, capitalisation or punctuation.
- **confirmed** requires two independent reads to agree: the
  full-image pass **and** a 3× row crop (or two adjacent runs'
  twins, e.g. `019`/`020`, which are pixel-identical per the harness).
- **uncertain** is mandatory when: the passes disagree; the text
  appears in only one pass; a word box is under ~7 px tall at 1×;
  the row was highlighted/selected (blue washes glyphs out); or you
  had to zoom with your eyes to see it. Always add the reason:
  `[uncertain: single-pass fragment, not reproduced in 3x crop]`.
- An unreadable region is reported as `[unreadable: <reason>]`, never
  as a best guess. Eye-reads may be mentioned **only** inside an
  `[uncertain: …]` note, never as the READ line.

---

## 5. Worked examples (real runs)

### Example 1 — confirmed: MISC popup menu rows + shortcut hints

File: `20261016-081527/captures/019_pe_misc_popup.png` (474×81,
class `#32768`). Command: `Ocr.exe` on the file, no crop (glyphs
11–18 px) [RAN — identical output from the s06r binary and a
clean-room rebuild compiled per §2a].

Verbatim engine output:

```
TEXT_START
Generate HTML playlist Read extended info on selection (Ctrl+Alt+G) (Ctrl+Alt+E)
TEXT_END
LINES: 4
LINE: Generate HTML playlist
WORD[54,12,69,13]: Generate
WORD[129,12,45,13]: HTML
WORD[180,11,54,18]: playlist
LINE: Read extended info on selection
WORD[54,57,37,14]: Read
WORD[98,57,71,14]: extended
WORD[176,57,29,14]: info
WORD[212,62,19,9]: on
WORD[238,57,67,14]: selection
LINE: (Ctrl+Alt+G)
WORD[347,11,93,17]: (Ctrl+Alt+G)
LINE: (Ctrl+Alt+E)
WORD[350,57,90,17]: (Ctrl+Alt+E)
```

Report (STATUS: **confirmed** — row 1 text at y≈11–24 with hint at
x≈347; row 2 text at y≈57–70 with hint at x≈350; separator band at
y≈40 yields no text, as expected):

- Row 1 left: `Generate HTML playlist` — right hint: `(Ctrl+Alt+G)`
- Row 2 left: `Read extended info on selection` — right hint: `(Ctrl+Alt+E)`

Note the `TEXT_START` blob concatenates both rows; attribution comes
from the `LINE:`/`WORD` records and their boxes, not from the blob.
No submenu arrows were legible — only the two shortcut hints.

### Example 2 — ambiguous: playlist row 3 of the list baseline

File: `20261016-081527/captures/025_pe_list_single_before.png`
(275×116). Full-image pass [RAN]:

```
LINE: tl
WORD[30,64,7,7]: tl
```

(no head, one 7×7 px fragment). The 3× row crop
(`(0,60,275,77)` → 3× NEAREST) returned **0 lines** [RAN].
A neighbouring-row check shows the head should exist (rows 0–2
confirmed as `57. longtitle`, `58. ref tone mono`,
`59. ref tone stereo`), and the eye sees something like `60.` plus
two more glyphs — but the engine never confirmed any of it.

Report (STATUS: **uncertain** — do NOT quote as a label):

```
FILE: 20261016-081527/captures/025_pe_list_single_before.png (full-image + (0,60,275,77) @3x)
READ (verbatim, engine output, uncorrected): "tl"
BOXES: WORD[30,64,7,7] (full-image pass only)
STATUS: uncertain
[uncertain: single-pass 7px fragment; 3x row crop returned 0 lines; eye suggests a "60." head
 that OCR never confirmed — treated as unreadable, not as "60. t1"]
```

This is the case the project rule is for: writing `60. t1` into a spec
would freeze a guess into an interface contract. `tl [uncertain]` keeps
the lane honest.

---

## 6. Limits — when OCR is the wrong tool, use pixel measurement

OCR reads **letterforms**. It cannot do any of the following; each of
these was met in this project's captures and each needs Pillow/a pixel
mask instead (as `s06r` did for the selection block):

- **Position of a check mark / submenu arrow / glyph.** `019` has no
  legible arrows; `007` line `> Dieser PC` shows the engine will happily
  OCR a navigation chevron as `>` — position and meaning need a human
  and a box, not a string.
- **Colour.** The selection rectangle is bright blue ≈ `(0,0,198)`;
  OCR is colour-blind. Scan for contiguous bands of the colour and
  measure their bounds (e.g. `026`: x≈12–254, y≈23–34, ~2707 px).
- **Whether a row is highlighted.** A selected row's glyphs wash out
  (cf. `028`: the selected row is "not legible, likely obscured by the
  highlight"). Absence of OCR text on a blue band is a selection
  signal, not an empty row.
- **Separators and lines.** The `019` band at y≈40 and menu rules are
  pixel geometry (full-width ~454 non-background px), not text.
- **Title bars.** `007`/`008` and `023`/`024` captures are client-area
  only — no caption text exists in the PNG. Titles (`Add file(s) to
  playlist`, `Load Playlist`) come from `bootstrap.log` window
  metadata, never from OCR.
- **Icon-only markers.** `007` lines `R Bilder`, `O`, `All supported
  types O`, `durchsuchen D` mix toolbar/tree glyphs (`R`, `O`, `D`,
  `C`) into the text — glyphs, not labels. Quote them verbatim, flag
  them, do not interpret them.
- **Localised chrome at small sizes.** German dialog strings misread
  deterministically: `etzwerk` (= `Netzwerk`), `vunamp` (= `winamp`),
  `Dieser pc` (= `Dieser PC`), `�ffnen` (= `Öffnen`, encoding artefact
  included). Verbatim + `[uncertain]` per §4; the correction, if
  needed, belongs in the lane's interpretation step, not in the READ
  line.
- **Anything under ~7 px.** Below one engine-recognisable glyph height
  the output is fragments (`tl`) or nothing — magnify per §3, and if
  it still fails, call it `[unreadable]`.

---

## 7. Anything the owner would have to approve

1. **Text-Grab install** — done by the owner: 4.15.0 is on `PATH` and its
   `--windowless` file CLI is tested (§2d). No approval needed for
   `--windowless` spot checks; bulk folder OCR opens an Edit Text window,
   so headless lanes must not use it. MiniSnip and standalone Tesseract
   add nothing over §2a and are **not** recommended even with approval.
2. **A shared home for the helper** — decided and done: the source and
   build script live in the repo (`tools/ocr/`), the canonical binary is
   `D:\code\tools\ocr\Ocr.exe` (§2a).

Nothing in this document required installing, downloading, running a
reference binary, starting a sandbox/VM, or reading
`rustamp-research\extracted\` or `rustamp-refbin`.

---

## A. Helper source (`tools/ocr/Ocr.cs`)

The canonical source lives in the repo at `tools/ocr/Ocr.cs` — it is
byte-identical to the listing formerly inlined here (verified [RAN
2026-10-16]) and builds per §2a. Prints text plus per-word bounding boxes
to stdout; first CLI argument is the PNG path.
