# SDK-licensing: what the Winamp plugin SDK's own terms say about a third-party host

Status: **licence reading, decision-supporting research note.** Research and analysis only. This note changes no code, crate, license file or manifest.

- Written by: dirty-room research subagent (spawned session), 2026-10-14.
- Question: does the **Winamp plugin SDK's own license** restrict what a non-Nullsoft host (`rustamp-plughost32.exe`) may do — implement the `in_`/`out_`/`vis_`/`dsp_`/`gen_` interfaces and load plugins built against the SDK?
- Context: RFC 0004 relicenses the plugin-facing crates permissively. That RFC answers the *GPL* question; it does **not** answer the *SDK* question, which this note covers.
- **This is a licence reading, not legal advice.** It records what named documents say. No court has been asked, and none of the readings below has been tested.
- Clean-room scope: this note contains no code, no pseudo-code, no header or struct content, no internal identifiers and no disassembly (CLEANROOM.md §4). It quotes only the short operative sentences that carry a permission or a restriction.

## 0. Method and labels

I examined the SDK material held in the dirty room and its public origin, plus the public web. I did **not** run, install or execute anything, and I did **not** download any file; everything below is a file already present, or a page fetched over HTTP.

| Label | Meaning |
|---|---|
| **[Grant]** | An explicit permission in the text. |
| **[Restriction]** | An explicit limit or condition in the text. |
| **[Silent]** | The document says nothing on the point. |
| **[No grant]** | Copyright applies by default; the document contains no permission. |
| **[Open question]** | Not established from the documents available. |

The SDK headers are a *mix*: one header carries a permissive, zlib-style notice, the rest carry no notice at all. That split is the single most important finding.

## 1. Documents examined

| # | Document | Origin | Identity |
|---|---|---|---|
| D1 | `wa_ipc.h` (modern Winamp IPC header, Winamp SA, 1997–2014) | dirty-room extraction of the Winamp 5.x source tree (`extracted/wasrc/wa_ipc.h`) | SHA-256 `eaa6080b360637d6ff010e0f07ee51f1f6dd30dec42273ef44efecb98f07d372` |
| D2 | `wa_ipc.h` (earlier copy, Nullsoft, 1997–2013) | Winamp SDK headers bundled in the MilkDrop 2.25c source zip (`extracted/mdsrc/Winamp/wa_ipc.h`) | SHA-256 `d1501234317016cd97fa6ffae7da68fbcac472fdd9cb7c28d5f5e8c951ecb588` |
| D3 | `FRONTEND.H` (classic Winamp 2.x frontend/IPC documentation, Nullsoft, 1997–2000) | dirty-room extraction of the 5.x source tree (`extracted/wasrc/FRONTEND.H`) | SHA-256 `d9d1fe0ea079d89281e628d33b9285f2a81ce3311a9ee836cbf155c6199ed701` |
| D4 | The five plugin interface headers: `IN2.H`, `OUT.H`, `DSP.H`, `VIS.H`, `GEN.H` | dirty-room extraction of the 5.x source tree (`extracted/wasrc/`) | SHA-256 `f61fb616…` (IN2.H), `d61bbc04…` (OUT.H), `23c9db33…` (DSP.H), `9d744950…` (VIS.H), `b0ff3378…` (GEN.H) |
| D5 | `VIS.H` as bundled in the MilkDrop 2.25c source zip | `extracted/mdsrc/Winamp/VIS.H` | SHA-256 `cbe9a4dcac8c93e9c7260359efbdebf5a160cb900f2f00c2396ae3b8ab183e1e` |
| D6 | `WA_SDK.nsi` — the NSIS build script of the Winamp 5.9 SDK installer | Winamp source release (community branch), `Src/installer/SDK/WA_SDK.nsi` | git blob id `17f2209ce0f576f37a5413ad033c4a825a5f42ee` |
| D7 | `winamp_api.nsh` — the SDK installer's header list | Winamp source release, `Src/installer/SDK/winamp_api.nsh` | git blob id `5d037b980af3d9c063ccda1f5180d3fc609d252b` |
| D8 | `WA5.9_SDK.exe` — the shipped Winamp 5.9 SDK installer | Winamp source release, `Src/installer/SDK/WA5.9_SDK.exe` (not extracted here) | git blob id `b785f09c1d04c2b191a91442a4ff38f7ae2d7368` |
| D9 | `LICENSE.md` — Winamp Collaborative License (WCL) v1.0 | Winamp source release (`extracted/winamp-src/LICENSE.md`) | SHA-256 `0077374a72023c96e25f5492d0c8a16ce9dcbf5d6d32efcbf61ebe7ed929da33` |
| D10 | NSDN classic SDK documentation pages (`…/nsdn/winamp2x/dev/sdk/`, `…/dev/plugins/in.php`) | Wayback Machine snapshots of `winamp.com`, 2003–2004 | page captures (no stable file hash) |
| D11 | Public statements: WACUP site and GitHub org; `heise`/`winaero` reporting on the 2024 release; the archived classic SDK file listing | public web, 2024–2026 | pages (no stable file hash) |

I did **not** obtain the contents of the classic Winamp 2.x mini-SDK packages themselves (`in_minisdk.zip`, `out_minisdk.zip`, `gen_minisdk.zip`, `dsp_minisdk.zip`, `vis_minisdk.zip`, archived at the old AOL/Nullsoft FTP path). See open question Q5.1.

## 2. The operative terms, in my own words

### 2.1 `wa_ipc.h` — the one header with a real license (D1, D2)

The modern IPC header opens with a short, zlib-style notice: copyright Nullsoft, Inc. (D2, 1997–2013) or Winamp SA (D1, 1997–2014); provided "as-is" with no warranty; and then an express grant to use, alter and redistribute the header freely for any purpose, including commercial use, subject to three conditions. The conditions are: do not misrepresent the origin or claim you wrote it (an acknowledgment in the product documentation is *appreciated but not required*); mark altered versions as altered and do not present them as the original; and do not remove or alter the notice.

This is a permissive, OSI-style free-software grant. It is **not** an end-user agreement, it names no host, it is not limited to Winamp, and it has no field-of-use limit. It is the same notice on both the 2.x-era and the 5.x-era copies.

### 2.2 The five interface headers — no notice at all (D4, D5)

`IN2.H`, `OUT.H`, `DSP.H`, `VIS.H` and `GEN.H` begin with an include guard and interface comments and contain **no copyright line, no permission and no restriction**. Their only "license" is therefore the default: copyright applies and no permission is granted by the header text itself. (The underlying ABI *facts* — sizes, offsets, constants, call order — are a separate matter; that is the subject of `docs/notes/L-licensing-plugins.md` and is not an SDK-license question.)

### 2.3 `FRONTEND.H` — copyright only (D3)

The classic frontend header is attributed and copyright-stamped ("Copyright (C) 1997-2000, Nullsoft Inc.") but contains **no permission grant**. It is the older document that `wa_ipc.h` supersedes.

### 2.4 The Winamp 5.9 SDK installer — headers plus examples, and no license file (D6, D7, D8)

The build script of the shipped SDK installer (D6) copies the API headers (D7 lists them: the IPC header, the dialog header, the DSP, general, playlist-IPC, input and output headers), the Wasabi/Agave/BFC API headers, and a set of example plugins. It contains **no license file, no EULA and no agreement page**; the only text file it references is an SDK readme, and that reference sits in a disabled block. The installer's own version resource carries a copyright line ("Copyright © 1997-2022 Winamp SA"), i.e. a notice, not a grant. So the 5.9 SDK's terms are exactly the terms of the individual files it ships (2.1–2.3), plus that copyright notice.

### 2.5 The NSDN documentation pages — a restrictive blanket notice (D10)

The classic SDK documentation pages on `winamp.com` (2003–2004) carried, in the page source, a site-wide notice to the effect that the pages' code and content are copyright Nullsoft and that redistribution or use of any content and code is strictly prohibited without AOL's express permission. This is a **web-page** notice, not the SDK package's own license, but it is the notice that sat next to the SDK download and documentation at the time. I did not establish whether the same notice appeared inside the mini-SDK packages.

### 2.6 The 2024 source release — the WCL (D9)

The 2024 release put the Winamp program source, and with it the SDK installer, into a repository governed by the **Winamp Collaborative License** (source-available, not OSI-approved). In my own words: you may view, study and privately modify; you may not distribute modified versions; v1.0 also forbade forking, and v1.0.1 removed that sentence (D11); contributions are assigned to Winamp; and no trademark rights are granted beyond describing the program's origin. The SDK headers keep their own notices (2.1–2.3); the WCL is the repository's terms.

## 3. The five questions

### Q1 — Does the SDK license permit a non-Nullsoft program to implement the interface and load plugins built against the SDK?

**Verdict: permitted** (for the header that carries a grant; the rest are silent, with no grant).

The IPC header is the only SDK file with an express grant, and it grants use "for any purpose, including commercial applications". It names no host and imposes no requirement that the user be Winamp:

> "Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions" — `wa_ipc.h` (D1, D2).

For the five interface headers (D4, D5) there is no grant in the text, so "permitted" rests on the copyright position on interface facts, not on an SDK permission — see open question Q1.1.

### Q2 — Does it impose conditions on the host (attribution, a licence agreement, a restriction to Winamp, a prohibition on third-party hosts, field-of-use limits)?

**Verdict: restricted** — but only mildly, and only as to redistributing the header; nothing binds the host's nature.

The three conditions in the grant are the only conditions, and they attach to copying/altering/redistributing the header, not to being a host:

> "1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required." / "2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software." / "3. This notice may not be removed or altered from any source distribution." — `wa_ipc.h` (D1, D2).

No licence agreement, no registration, no restriction to Winamp, no prohibition on third-party hosts, and no field-of-use limit appears anywhere in the SDK material (D1–D8). Commercial use is expressly allowed.

### Q3 — Redistributing the SDK headers; reverse engineering; the "Winamp" name/trademark

Three sub-questions, three verdicts:

- **Redistributing the headers — restricted / mixed.** Redistributing `wa_ipc.h` is expressly permitted with the three conditions above (D1, D2). The other five headers carry no grant, so their redistribution is not licensed by the header text (**[No grant]**). The NSDN page notice added a blanket restriction on the documentation pages: "Redistribution or use of any content and code is strictly prohibited without the expressed permission America Online, Inc." (D10). And the repository copy sits under the WCL, which forbids distributing modified versions: "You may not distribute modified versions of the software, whether in source or binary form." (D9).
- **Reverse engineering — silent.** Neither the SDK grant (D1, D2) nor the WCL (D9) mentions reverse engineering, decompilation or interoperability. (The *player's* end-user agreement is a separate document and was not the SDK; I did not rely on it.)
- **"Winamp" name / trademark — silent in the SDK; a clause in the WCL.** The SDK headers contain no trademark or compatibility clause, even though interoperability requires naming the Winamp window class (D1, D3). The WCL reserves the marks: "This License does not grant permission to use the trade names, trademarks, service marks, or product names of the Licensor, except as required for describing the origin of the Program and reproducing the content of the copyright notice." (D9). WACUP's own site relies on a "fair usage" description of the marks (D11).

### Q4 — Is there a separate plugin developer agreement or EULA that applies to plugin authors rather than hosts?

**Verdict: could not determine (none found).** I found no plugin-developer agreement, no SDK EULA and no click-through terms for the SDK download. The 5.9 SDK installer ships no such file (D6–D8); the archived NSDN SDK pages present no acceptance step (D10). The only EULA in the ecosystem I located is the *player's*, which is not the SDK and was not used as a basis for any verdict here. See open question Q4.1.

### Q5 — Is the classic SDK still available, and under which terms today? Did the 2024 release change anything? What do third-party hosts say?

**Verdict: available; terms = the per-header notices above; the 2024 release added the SDK to a WCL-governed repository.**

- The classic Winamp 2.x headers (the `in_`/`out_`/`vis_`/`dsp_`/`gen_` set plus the frontend/IPC header) remain available: they are archived on the Wayback Machine (D10) and they are the same headers carried forward into the 5.x SDK, which is still shipped in the 2024 source release as `Src/installer/SDK/WA5.9_SDK.exe` (D8) and mirrored publicly (D11).
- **The 2024 release changed one relevant thing:** the SDK installer and headers are now part of the WCL-governed repository (D9), so the *repository copy* is under the WCL (no distribution of modified versions; v1.0 also banned forking, v1.0.1 dropped that sentence). The headers' own notices (2.1–2.3) are unchanged.
- **Third-party hosts.** WACUP is the strongest public precedent: an independently developed, Winamp-compatible host that loads classic plugins, whose site states it "is not officially associated with Winamp" and treats the marks as third-party property used under fair usage (D11). WACUP also maintains a repository of archived Winamp SDK revisions (D11). I found **no** public legal statement by WACUP or another host that expressly analyses the SDK's license; their position is practical, not a published reading.

## 4. Open questions

1. **Q1.1 — the five headers have no grant.** `IN2.H`/`OUT.H`/`DSP.H`/`VIS.H`/`GEN.H` carry no permission. Whether a third-party host may *copy and redistribute* those header files (as opposed to implementing the ABI from the published facts) is not answered by the headers. `docs/notes/L-licensing-plugins.md` covers the "facts are not copyrightable" line; this note does not re-derive it.
2. **Q5.1 — classic mini-SDK package contents not inspected.** The 2.x `*_minisdk.zip` packages are archived but were not opened (no downloads). Whether they contained a readme or a license file — and whether the NSDN page notice (2.5) was also inside them — is unverified.
3. **Q4.1 — no plugin-developer agreement found, but absence is not proof.** I searched the SDK installer, the source release and the archived NSDN pages. A click-through agreement could have existed at some earlier point or on a page not archived; I could not confirm one did not.
4. **Q5.2 — WCL vs. per-header grant.** The WCL now covers a repository that also contains the permissively-noticed `wa_ipc.h`. Which instrument governs that header when it is redistributed *out of* the repository is an unresolved conflict; the more specific per-header grant is the natural reading, but the WCL's "no distribution of modified versions" is not obviously reconciled with it.
5. **Q5.3 — trademark and the compatibility name.** Interoperability requires naming the Winamp window class, and the WCL grants no trademark rights except to describe origin. Whether a third-party host's use of the compatibility name is covered by that exception or by fair usage is a trademark question this note does not settle.

## 5. Limits of this note

- It is a **licence reading**, not legal advice, and it is confined to the documents in §1. Anything not in §1 is marked as an open question rather than guessed.
- It deliberately does **not** reproduce any license in full, and it contains no code, headers, struct definitions, internal identifiers or disassembly.
- The GPL-boundary question (whether Rustamp's own licensing reaches the plugins) is out of scope here; it lives in `docs/notes/L-licensing-plugins.md` and RFC 0004.
