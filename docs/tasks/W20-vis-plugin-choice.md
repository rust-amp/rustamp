# W20 — Which open-source MilkDrop-like visualisation to ship (candidate choice only)

**Date:** 2026-10-18. **Status:** decision proposal + verified download. **Nothing is
bundled, wired or recorded in the licence inventory by this card.**

This is step 1 of the owner-approved "ship an open-source MilkDrop-like
visualisation" work: pick a component, prove its licence against *this* project's
licence split, acquire the artefact outside the repository, and stop. Bundling,
wiring (`rustamp-plughost32.exe` / `rustamp-milkdrop`), the `THIRD_PARTY.md`
entry, the X10 package layout change and any build are explicitly **not** started.

**Boundary actually observed.** Worktree
`C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave94-plugin`, initial
`git status --short` empty at `f52645a`. The only repository file written is this
card. No dependency added, no `Cargo.toml`/`Cargo.lock` touched, no `cargo test`,
no VM, no build, no install, no git write, no commit. Staging lives outside every
checkout at `D:\code\rustamp-vm\staging\vis-plugin`. The dirty Winamp SDK headers
inside the downloaded archive were **not opened** (CLEANROOM.md §1); their licence
state was determined by a classifier that printed a verdict, not their contents.

---

## 1. What the project's own licensing actually permits

Three instruments decide this, and they do not say the same thing:

| Instrument | What it binds | Consequence for a third-party visualisation |
|---|---|---|
| `LICENSE` (GPL-3.0-or-later) + `THIRD_PARTY.md` "Licensing of Rustamp itself" | The player and every GPL crate, incl. `rustamp-vis`, `rustamp-milkdrop` | A bundled component must be GPL-3.0-or-later **compatible**. GPL-3.0 §10 also forbids imposing any further restriction on recipients, which kills every "no redistribution of modified versions" licence. |
| `PLUGIN-EXCEPTION.md` (GPL-3.0 §7 additional permission) | Rustamp's **own** GPL code on the player↔plugin-host edge | Explicit limits: *"This exception does not apply to material in the covered work that is licensed under the GPL by third parties"* and, in §Notes, *"A GPL-compatible dependency is not automatically plugin-exception-compatible … do not put copyleft in the permissive host's dependency closure."* So GPL-compatibility of a candidate is **not** sufficient. |
| `MIT OR Apache-2.0` on `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost` + `CONTRIBUTING.md` §Dependencies | The DLL-loading host and the shipped `host/` package | *"only permissive licenses are allowed throughout their normal dependency closures"*; *"no dependency of the DLL-loading host may impose copyleft on a plugin; our GPL exception cannot cure that."* X10's `audit_host_dir` enforces this mechanically: a copyleft/MPL token in `host/` fails `cargo xtask package --check` (W16-C3, **Bestätigt**). |

Two further hard constraints come from `CLEANROOM.md` §1, which already pre-rules
on exactly this question:

- `DeaDBeeF, webamp, butterchurn, WDL/EEL2, libopenmpt, projectM (LGPL, reference only)` → copy allowed with attribution, but §7(4) adds *"projectM (LGPL, reference only; **do not copy from projectM without a separate review**)"*.
- `MilkDrop 2.25c source (geisswerks.com, milkdrop_225c_src.zip): files under vis_milk2/ that carry the Nullsoft BSD-3 header` → copy allowed with attribution, **except** the bundled SDK headers (`Winamp/`, `Wasabi/`, `Agave/`, `vis_milk2/vis.h`, `api.h`) and *any file without a BSD header*, which are dirty; `ns-eel2/` licence to be checked separately.

And one architectural constraint from `specs/S09-plugin-abi.md` §5: a classic
visualisation is a 32-bit x86 `winampVisGetHeader` DLL with header version `0x101`
and a 2360-byte module, fed 2 × 576 spectrum and 2 × 576 waveform bytes. Only
`rustamp-plughost32.exe` can load such a DLL; the 64-bit player never does.

---

## 2. Candidates, exact licences, upstream homes, and the clause that decides

### 2.1 MilkDrop 2.25c — `vis_milk2` (**recommended**)

- **Upstream home:** https://www.geisswerks.com/milkdrop/ (Ryan Geiss, author's own
  site; the page offers `milkdrop_225c_src.zip`, 15 May 2013, and says "Also
  available via git at SourceForge"). Project home:
  https://sourceforge.net/projects/milkdrop2/ (owners `geissomatik`, `rovastar`).
- **Licence, exact:** **BSD-3-Clause**, verified in the downloaded archive, e.g.
  `vis_milk2/vis.cpp`: *"Copyright 2005-2013 Nullsoft, Inc. All rights reserved.
  Redistribution and use in source and binary forms, with or without modification,
  are permitted provided that the following conditions are met: … Neither the name
  of Nullsoft nor the names of its contributors may be used to endorse or promote
  products derived from this software without specific prior written permission."*
  31 of the 47 `vis_milk2/` entries carry this header.
- **Second licence inside the same archive:** `ns-eel2/` (the MilkDrop expression
  engine) is **zlib**, verified in `ns-eel2/ns-eel.h`: *"This software is provided
  'as-is' … Permission is granted to anyone to use this software for any purpose,
  including commercial applications, and to alter it and redistribute it freely,
  subject to the following restrictions: 1. The origin of this software must not be
  misrepresented … 2. Altered source versions must be plainly marked as such …"*
  (Nullsoft 1999-2003 / Cockos 2004-2008), 11 of 15 entries.
- **Deciding clause:** BSD-3-Clause has **no reciprocal term at all**. Its only
  conditions are notice retention (clauses 1 and 2) and the no-endorsement clause
  3. Nothing propagates into a dependency closure. Therefore it satisfies *both*
  the GPL player (BSD-3 is GPL-compatible) *and* `CONTRIBUTING.md`'s
  permissive-only rule for the host closure, and `audit_host_dir` has nothing to
  reject. zlib behaves the same way.
- **Verdict: can be bundled with the player AND bundled/hosted on the permissive
  plugin-host side.** It is the only candidate that is clean on both sides.
- **It is also the only candidate that already *is* an S09 §5 visualisation
  plugin**: the sources build `vis_milk2.dll`, i.e. the exact `winampVisGetHeader`
  / 2360-byte-module shape the plughost was specified against (S09 §5.2 even
  records MilkDrop's `latency_ms` 0 / `delay_ms` 10).

**What is *not* usable in that archive** (found by inspecting the download, §4):

| Path | State | Why it blocks verbatim redistribution |
|---|---|---|
| `Winamp/VIS.H`, `Winamp/wa_ipc.h`, `Wasabi/**` (9), `Agave/Language/**` (2), `vis_milk2/vis.h`, `vis_milk2/api.h` | **Dirty** (CLEANROOM.md §1) | Winamp SDK headers. Not opened by this card. Must be pruned and replaced by `rustamp-plugin-abi`-derived clean declarations before any clean-room implementer touches the tree. |
| `vis_milk2/d3dx9.lib` (5,959,648 B), `vis_milk2/d3d9.lib`, `vis_milk2/vms_desktop.lib` | **No licence notice**; first two carry Microsoft DirectX 9 SDK library names | Prebuilt Microsoft binaries. Not open-source material; redistributing them from this archive is not permitted by any licence we hold. |
| `ns-eel2/asm-nseel-x86-msvc.c`, `asm-nseel-x86-gcc.c`, `asm-nseel-ppc-gcc.c`, `asm-nseel-x64-macho.o` | **No licence notice** → dirty by CLEANROOM.md §1 | This is exactly open question 4 in `docs/notes/L-licensing-plugins.md` §5, now confirmed against the artefact. A pure-C fallback or our own code is needed instead. |
| `nu/AutoCharFn.h`, `nu/AutoWide.h`, `nu/Vector.h`, `vis_milk2/DOCUMENTATION.TXT` | No licence notice / copyright without a grant | Dirty; not usable. |

So the honest shape of the recommendation is: **ship a plugin built from the
BSD-3-Clause `vis_milk2/` subset plus the zlib `ns-eel2/` subset, never the archive
as downloaded.** It needs a 32-bit MSVC build, a Direct3D 9 path, and replacement
of the SDK headers and the four unlicensed asm files. That cost is real and is the
main argument against this candidate.

### 2.2 projectM (libprojectM) — the maintained alternative

- **Upstream home:** https://github.com/projectM-visualizer/projectm .
  Latest release **v4.1.8**, published 2026-10-06, source tarball
  `libprojectM-4.1.8.tar.gz` (1,309,526 B, GitHub-recorded digest
  `sha256:f72135e8b8e5fd807f908537255b41ac7ba4830309265e830cb756535487239e`).
- **Licence, exact:** **LGPL-2.1-or-later**. `LICENSE.txt` is the LGPL 2.1 text;
  the `COPYING` header reads *"This library is free software; you can redistribute
  it and/or modify it under the terms of the GNU Lesser General Public License as
  published by the Free Software Foundation; either version 2.1 of the License, or
  (at your option) any later version."* (The README's bare "LGPL 2.1" wording omits
  the "or later"; the file headers carry it.)
- **Deciding clauses, both directions:**
  - *Bundling with the GPL player is permitted.* LGPL-2.1 §3: *"You may opt to
    apply the terms of the ordinary GNU General Public License instead of this
    License to a given copy of the Library… (If a newer version than version 2 of
    the ordinary GNU General Public License has appeared, then you can specify that
    version instead if you wish.)"* — so the library converts cleanly to GPL-3.0,
    and §6 separately allows distributing a combined work under our own terms if
    the source/relink obligations are met.
  - *Bundling it on the permissive host side is not permitted by our own rules.*
    LGPL-2.1 §2(c) and §6 impose reciprocal source and relink obligations;
    `CONTRIBUTING.md` §Dependencies allows *"only permissive licenses … throughout
    their normal dependency closures"* for the five permissive crates, and
    `PLUGIN-EXCEPTION.md` §Notes says *"do not put copyleft in the permissive
    host's dependency closure."* X10's `audit_host_dir` would fail the package.
- **Verdict: bundleable with the player only.** A user may of course install a
  projectM-based DLL themselves — the MIT host imposes nothing on any plugin and
  LGPL permits being loaded — but it must stay out of the shipped `host/` package.
- **Second problem:** projectM 4.x is a library, not a plugin. Upstream's own
  README lists the Winamp front-end among *"legacy sources for ALSA, XMMS, Winamp,
  and JACK (unmaintained since 2012)"*, so the `vis_` wrapper would have to be
  written here anyway, on top of LGPL §6 relink obligations for the release.
- **Third problem:** `CLEANROOM.md` §7(4) requires *"a separate review"* before any
  code is copied from projectM. Linking/FFI is not copying, but the review is still
  a gate the owner has to open.

### 2.3 Butterchurn — licence-perfect, but not a plugin

- **Upstream home:** https://github.com/jberg/butterchurn ; npm `butterchurn`
  (latest `2.6.7`, 2019-03-12; `3.0.0-beta.5` on the beta tag).
- **Licence, exact:** **MIT** (`LICENSE`: *"MIT License / Copyright (c) 2013-2018
  Jordan Berg"*), also declared `"license": "MIT"` in the npm manifest.
- **Deciding clause:** MIT's only condition is *"The above copyright notice and
  this permission notice shall be included in all copies or substantial portions of
  the Software"* — no reciprocity, so it is clean for the player **and** for the
  permissive host closure.
- **Verdict: licence-wise it may be bundled anywhere — but it answers neither ask.**
  It is JavaScript/WebGL. It cannot be an S09 §5 32-bit `vis_` DLL, and shipping a
  JS engine in the release is not on the table. Its real role is already recorded:
  `crates/rustamp-milkdrop` is described as a *"Native MilkDrop 2 port
  (butterchurn-derived)"*, and `docs/notes/L-licensing-plugins.md` §3.7 lists it as
  *"copied/ported into `rustamp-milkdrop` (M4)"*. It stays the **port source**, not
  the shipped component.

### 2.4 Winamp 5.x's own MilkDrop 2 — **neither**

- **Upstream:** the `vis_milk2.dll` shipped inside `winamp566_full_en-us.exe`, and
  the Winamp 5.x source published on GitHub in September 2024 (repository since
  deleted).
- **Licence, exact:** the source release used the **Winamp Collaborative License
  (WCL) 1.0.1**; the bundled binaries are closed-source freeware with no published
  redistribution grant.
- **Deciding clause:** WCL 1.0.1 *"No Distribution of Modified Versions: You may not
  distribute modified versions of the software, whether in source or binary form."*
  That is a further restriction, so it fails GPL-3.0 §10 for the player, fails the
  permissive host package, and fails `CLEANROOM.md` §1, which classifies Winamp 5.x
  source as **dirty room only**. `docs/notes/L-licensing-plugins.md` §5 open
  question 2 already flags the 5.66 MilkDrop binary's terms as unverified.
- **Verdict: can be neither bundled with the player nor shipped on the plugin-host
  side.** Users installing it themselves remains their own affair, and that is
  exactly what the permissive host exists for.

### 2.5 Summary table

| Candidate | Exact licence | Bundle with GPL player? | Ship in permissive `host/` / as bundled plugin? | Deciding clause |
|---|---|---|---|---|
| **MilkDrop 2.25c `vis_milk2/`** | BSD-3-Clause (Nullsoft 2005-2013) | **Yes** | **Yes** | BSD-3 has no reciprocal term; only notice + no-endorsement |
| MilkDrop 2.25c `ns-eel2/` | zlib (Nullsoft/Cockos) | Yes | Yes | zlib: origin/marking/notice only |
| MilkDrop 2.25c SDK headers, `d3d*.lib`, 4 asm files, `nu/` | dirty / none | **No** | **No** | No grant at all; CLEANROOM.md §1 |
| **projectM 4.1.8** | LGPL-2.1-or-later | **Yes** (LGPL-2.1 §3 → GPL-3.0) | **No** | LGPL §2/§6 reciprocity vs. CONTRIBUTING.md permissive-only closure + `audit_host_dir` |
| Butterchurn 2.6.7 | MIT | Yes | Yes | MIT notice-only — but it is not a DLL; port source for `rustamp-milkdrop` |
| Winamp 5.x MilkDrop 2 (WCL 1.0.1 / freeware binary) | WCL 1.0.1 / proprietary | **No** | **No** | WCL "No Distribution of Modified Versions" vs. GPL-3.0 §10; CLEANROOM.md §1 dirty |

---

## 3. Recommendation

**Ship MilkDrop 2.25c (BSD-3-Clause `vis_milk2/` + zlib `ns-eel2/`).**

Reasons, in the order that decided it:

1. **It is the only candidate whose licence clears both halves of our split.**
   BSD-3 and zlib carry no reciprocity, so the component can sit next to the GPL
   player *and* inside the permissive `host/` package without touching
   `PLUGIN-EXCEPTION.md`'s limits or failing `audit_host_dir`. projectM forces a
   player-only bundle plus LGPL §6 relink obligations on every release.
2. **It is already pre-cleared by our own clean-room policy.** `CLEANROOM.md` §1
   names this exact archive and this exact subset as copy-allowed for everyone,
   with the dirty exceptions spelled out. projectM still needs the "separate
   review" that §7(4) demands.
3. **It is the real thing, in the shape the host already specifies.** S09 §5 was
   written against this plugin class; S09 §5.2 even records MilkDrop's own
   `latency_ms`/`delay_ms`. projectM 4.x would need a `vis_` wrapper written from
   scratch anyway, since its Winamp front-end is unmaintained since 2012.
4. **It converges with the existing plan** rather than adding a parallel one:
   `crates/rustamp-milkdrop` already targets a native MilkDrop 2 port, and
   `docs/notes/L-licensing-plugins.md` §3.7 already books MilkDrop 2.25c and
   ns-eel2 as copied sources for M4.

**Against it, stated plainly:** the code is 2013-vintage Direct3D 9 and 32-bit
MSVC, it is unmaintained, and the archive cannot be used as downloaded — the SDK
headers must be replaced by `rustamp-plugin-abi`-derived declarations, the
Microsoft `d3d*.lib` files must not be redistributed, and the four unlicensed
`ns-eel2` asm files must be dropped. If the owner prefers a maintained, actively
released, OpenGL, dirty-free codebase and accepts a **player-only** bundle with
LGPL §6 obligations and a separate CLEANROOM §7(4) review, **projectM 4.1.8** is
the fallback and nothing else here changes.

---

## 4. The artefact that was downloaded

Staging, outside every checkout: `D:\code\rustamp-vm\staging\vis-plugin\`
(`PROVENANCE.md`, `SHA256SUMS`, `MD5SUMS`, `milkdrop_225c_src.zip`).

| Field | Value |
|---|---|
| File | `milkdrop_225c_src.zip` |
| Component / version | MilkDrop **2.25c** source release, open-sourced 2013-05-14 |
| URL | `https://www.geisswerks.com/milkdrop/milkdrop_225c_src.zip` |
| Provenance | Author's own site (Ryan Geiss, geisswerks.com), which links SourceForge as the project home |
| HTTP `Last-Modified` | `Thu, 16 May 2013 03:34:11 GMT` |
| Size | 2,367,952 bytes (95 zip entries) |
| **SHA-256** | `4e1bce819db8c0e43a41fba309f197e50502f6f00c4bd148fb6b86dac6aa4c45` |
| SHA-1 | `9abd4c1bb9ffada1da86ab7e479c8cf8d55d64b4` |
| MD5 | `23ce630112916f63b34dcbce23cd2801` |

**Upstream checksum check: passed.** geisswerks.com publishes no checksum, but the
SourceForge project's file feed (`https://sourceforge.net/projects/milkdrop2/rss?path=/`)
publishes `filesize="2367952"` and `md5 = 23ce630112916f63b34dcbce23cd2801` for the
same release under the name `milkdrop2_v2.25c_OPEN_SOURCED_20130514_orig_code.zip`.
The downloaded archive matches both exactly, so the author's-site archive is
byte-identical to the official SourceForge release and the download is verified
against a published upstream checksum. No upstream SHA-256 exists; the SHA-256
above is ours.

The archive was only listed and licence-classified. It was **not** extracted into
the repository, not built, not installed, and the dirty paths were not opened.

---

## 5. What the owner must approve before step 2 begins

1. **The component and its licence**: MilkDrop 2.25c, BSD-3-Clause `vis_milk2/` +
   zlib `ns-eel2/` — or the projectM 4.1.8 fallback with its player-only limit.
2. **Which package it ships in**: `player/`, `host/`, or a new `Plugins/` folder.
   This changes X10's strict whitelist in `audit_package_layout` and the
   `audit_host_dir` expectations, and the W16-C3 audit must stay red for anything
   unexpected.
3. **Confirmation that the three unusable groups are dropped**, not worked around:
   the Winamp SDK headers (replaced by `rustamp-plugin-abi`-derived declarations),
   the Microsoft `d3d9.lib`/`d3dx9.lib`/`vms_desktop.lib` binaries, and the four
   unlicensed `ns-eel2` asm files.
4. **The CLEANROOM.md §7(4) separate review**, if the owner picks projectM.
5. **The `THIRD_PARTY.md` entry and the release notices** — a new copied-source
   row, the BSD-3-Clause and zlib full texts in the generated notices, and an
   update to the "Projects we expect to draw from" line. This also lands next to
   W16-F08, which is still open: the generated `player/THIRD_PARTY.md` does not
   actually carry the MPL-2.0 and Apache-2.0 full texts it claims.
6. **A build decision**: a 32-bit MSVC + Direct3D 9 toolchain in the packaging
   path, which X10's `cargo xtask package` does not have today.

Until those are approved, no file outside this card and the staging directory may
be touched.
