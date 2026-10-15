# Plugin authors' guide: under what terms your classic plugin runs with Rustamp

> **This is not legal advice.** This document restates what the Rustamp
> project's own notes and RFCs say about its licensing. It states no legal
> conclusion of its own, it is not a licence grant, and it cannot grant
> anything for code the project does not own. The GNU project says the same of
> its own publications: "You can use our publications to understand how GNU
> licenses work or help you advocate for free software, but they are not legal
> advice. The FSF cannot give legal advice."
> (`docs/notes/L-licensing-plugins.md`, §1.2, quoting the GNU GPL FAQ). If the
> answer matters to you commercially, have a lawyer read your own plugin's
> licence and your own distribution plans.
>
> Quotations below follow the wording of the notes they are taken from; the
> notes' typographic emphasis (bold markers) inside quoted text is not
> reproduced.

Written for: authors and distributors of classic Winamp 2.x plugins
(`in_`/`out_`/`vis_`/`dsp_`/`gen_`) who want to know whether Rustamp can run
their plugin, and on what terms. Sources: `docs/notes/L-licensing-plugins.md`,
`docs/notes/SDK-licensing.md`, `docs/rfc/0004-plugin-licensing.md` (with its
§7 errata), `PLUGIN-EXCEPTION.md`, `README.md` (License section) and
`THIRD_PARTY.md`. Nothing here is taken from Winamp source, SDK headers or the
Winamp binary — see [CLEANROOM.md](../../CLEANROOM.md).

Sections marked **[provisional]** go beyond what those notes state, or rest on a
description the notes give second-hand. They are marked rather than asserted.

---

## 1. The short version for every plugin author

Whatever licence your plugin carries:

- **Running is unrestricted.** The GPL "explicitly affirms your unlimited
  permission to run the unmodified Program" (GPL-3.0 §2, quoted in
  `docs/notes/L-licensing-plugins.md` §1.1). GPL conditions attach to
  *distributing* covered works, not to *executing* them.
- **No copyleft reaches your plugin from Rustamp.** The process that loads
  your DLL is licensed `MIT OR Apache-2.0`, so no combined-work theory can
  start on the plugin edge (RFC 0004 §1–§2).
- **Your own licence still governs your own code.** Rustamp's licensing says
  nothing about what you may do with *your* plugin; that is your licence's
  business.

## 2. The five plugin kinds, and where they run

The classic plugin kinds are the ones the host implements:

| Kind | Role |
|---|---|
| `in_` | input / decoder plugin — opens a file or stream and hands audio to the player |
| `out_` | output plugin — receives the audio and plays it |
| `vis_` | visualization plugin — receives the sample/spectrum frame |
| `dsp_` | DSP plugin — a stage that transforms the audio in the chain |
| `gen_` | general plugin — an extra window or service that hooks into the player |

**Where they run.** A classic plugin DLL is never loaded by `rustamp.exe`.
The 64-bit player (GPL-3.0-or-later) spawns a separate 32-bit process,
`rustamp-plughost32.exe`, and talks to it over a versioned IPC protocol (a
control channel plus shared-memory rings). The plugin host loads the
`in_`/`out_`/`vis_`/`dsp_`/`gen_` DLLs **into its own address space** and
presents the classic host surface to them — a fake "Winamp v1.x" main window
and the classic IPC messages — as recorded in
`docs/notes/L-licensing-plugins.md` §1.6. The split is required by bitness:
classic plugins are 32-bit x86 DLLs, and a 64-bit process cannot load them. It
also gives crash isolation, so a crashing plugin does not take down the player.

**The host's licence.** `rustamp-plughost32.exe`, `rustamp-plugin-abi`,
`rustamp-ipc` and `rustamp-proto` are licensed **MIT OR Apache-2.0**; every
other crate, including the player, is **GPL-3.0-or-later** and carries a
GPL-3.0 §7 additional permission (README, License section; RFC 0004 §1;
`THIRD_PARTY.md` → "Licensing of Rustamp itself"). The host binaries ship
inside the same installer as the player; that juxtaposition is, in the notes'
terms, a mere aggregation of separate programs (GPL-3.0 §5).

**Rustamp does not ship plugins.** Plugins are user-installed; the project
bundles and redistributes none (RFC 0004 §5; `docs/notes/L-licensing-plugins.md`
§1.6(4)). That is what makes the "run both kinds" story work without any
per-plugin decision.

## 3. If your plugin is proprietary

**What you may do.**

- Ship and sell your plugin under whatever terms you like. The notes' reading
  is that a plugin loaded into the permissively licensed host creates no
  copyleft obligation in either direction, so no obligation flows from Rustamp
  to your code (`docs/notes/L-licensing-plugins.md` §1.6(1), §3.3).
- Reuse `rustamp-plugin-abi` in your own tools: it is `MIT OR Apache-2.0`, and
  the notes point at DeaDBeeF's permissively licensed plugin API header as the
  model for that choice (`docs/notes/L-licensing-plugins.md` §2.9, §3.3).
- Expect to be loaded like any other classic plugin. Rustamp adds nothing to
  your obligations, and asks nothing of you: no registration, no agreement, no
  fee is described anywhere in the project's notes.

**What you must not do.**

- **Do not copy or redistribute the Winamp SDK's interface headers.** The five
  interface headers (`IN2.H`, `OUT.H`, `DSP.H`, `VIS.H`, `GEN.H`) carry **no
  permission at all** — no grant, no licence — so redistribution is not
  licensed by the header text (`docs/notes/SDK-licensing.md` §2.2, Q1.1, Q3).
  Implement the interface, or link your plugin against your own build of it;
  do not re-post the headers.
- **Do not rely on redistributing the SDK documentation.** The classic NSDN
  documentation pages carried a blanket notice: "Redistribution or use of any
  content and code is strictly prohibited without the expressed permission
  America Online, Inc." (`docs/notes/SDK-licensing.md` §2.5).
- **The one header that does carry a grant has conditions.** `wa_ipc.h` is
  permissively licensed ("Permission is granted to anyone to use this software
  for any purpose, including commercial applications, and to alter it and
  redistribute it freely, subject to the following restrictions"), and the
  three conditions are about copying it, not about being a host: do not
  misrepresent its origin; mark altered versions as altered; and
  "This notice may not be removed or altered from any source distribution."
  (`docs/notes/SDK-licensing.md` §2.1, Q2–Q3). Attach them only if you
  redistribute that header.
- **Do not use the Rustamp or Winamp names as an endorsement.** Rustamp ships
  a trademark notice stating it "is not affiliated with, endorsed by or
  sponsored by" the owner of Winamp (README, Trademark notice). Whether a
  third-party *host* may name the compatibility interface is itself an open
  trademark question in the notes (`docs/notes/SDK-licensing.md` Q5.3) — it
  says nothing about your plugin's own naming, which stays your call and your
  risk.
- The 2024 Winamp source release put the SDK inside a repository under the
  Winamp Collaborative License, which says "You may not distribute modified
  versions of the software, whether in source or binary form."
  (`docs/notes/SDK-licensing.md` §2.6, Q3, Q5.2). That instrument governs the
  repository copy; how it interacts with the per-header grant when a header is
  redistributed *out of* that repository is an unresolved conflict in the note
  (Q5.2), i.e. another reason not to redistribute SDK files.

**Support expectations.**

- Your plugin runs in a separate process: a crash is contained, and the host
  can be restarted without taking the player down (README; RFC 0004 §4.5). That
  is a robustness property, not a compatibility promise.
- Compatibility with a given plugin is tracked as a test matrix, not as a
  per-plugin guarantee (`docs/notes/L-licensing-plugins.md` §3.1).
- The surface a plugin may rely on is the documented classic ABI. The
  author-facing API document does not exist yet — see §8.

## 4. If your plugin is GPL (and the GPL version matters)

- **Your licence governs your code.** The §7 plugin exception described in §6
  is a permission granted by Rustamp's copyright holders **over Rustamp's own
  code**. It grants nothing over, and imposes nothing on, your plugin — and it
  could not: "if you want to use parts of other GPL-covered programs by other
  authors in your code, you cannot authorize the exception for them. You have
  to get the approval of the copyright holders of those programs." (FSF FAQ
  #GPLIncompatibleLibs, quoted in `docs/notes/L-licensing-plugins.md` §1.2,
  §3.2.) Only you can speak for your plugin.
- **GPL-3.0 / GPL-2.0-or-later plugins run normally.** Your plugin keeps its
  own copyleft and its own distribution duties; Rustamp neither helps nor
  hinders them.
- **GPL-2.0-only plugins run too, and that is the interesting case.** GPL-2.0
  and GPL-3.0 code cannot be combined ("if you tried to combine code released
  under GPLv2 with code under GPLv3, you would violate section 6 of GPLv2."
  — FSF FAQ #v2v3Compatibility, quoted in `docs/notes/L-licensing-plugins.md`
  §1.5). Rustamp does not have to solve that here, because **the process that
  loads your DLL is not GPL at all**: no combination forms, so no version
  conflict arises. This is exactly why the host is permissively licensed rather
  than merely excepted (§1.5, §3.3).
- **Rustamp's §7 exception is not a grant for your code and cannot be made
  one by us.** It is a permission over Rustamp's own code; whether your plugin
  may be distributed alongside that code is decided by *your* licence, which
  only you can relax. Treat §6 as an explanation of our side of the boundary.

## 5. If your plugin uses another open-source licence

This covers permissive licences (MIT/BSD/zlib/Apache-2.0), weak copyleft
(LGPL), and free-but-GPL-incompatible licences (the FSF's list includes, among
others, AGPLv1, the Academic Free License and Apache-1.0/1.1 —
`docs/notes/L-licensing-plugins.md` §1.5, §3.7).

- **Rustamp treats your plugin exactly as in §3**: the host carries no
  copyleft, so the licence you chose for your plugin stays the only licence
  that speaks to your plugin.
- **Your obligations are to your own upstream, not to Rustamp.** Attribution,
  source availability, notice retention and patent terms are yours to keep;
  Rustamp does not intercept, relicense or redistribute your binary.
- If your plugin is LGPL, note only that LGPL's linking permissions are about
  *linking a library*, which is not what a host loading a plugin DLL is
  (`docs/notes/L-licensing-plugins.md` §3.4). It changes nothing here: you are
  already outside the host's copyleft.
- One census caveat: the notes could not establish how many classic plugins are
  in each class, so each plugin's own notice must be read rather than assumed.
  An unversioned "GPL" notice is ambiguous (GPL-1.0+? GPL-2.0+?)
  (`docs/notes/L-licensing-plugins.md` §1.5, §5 open questions 2 and 3).

## 6. The GPL-3.0 §7 plugin exception, in one paragraph

Rustamp's GPL-licensed crates carry an **additional permission under section 7
of the GNU GPL, version 3**, so that combining them with modules that speak the
documented plugin ABI or the documented IPC protocol needs no further grant.
The permission covers only Rustamp's own code, only through those documented
interfaces, and only while those interfaces stay unmodified; it does not apply
to third-party GPL material inside a covered crate. It is a *permission*, not a
licence: anyone who conveys a copy may remove it, so it is a companion to the
permissive host licence, not a substitute for it. The operative text is in
[PLUGIN-EXCEPTION.md](../../PLUGIN-EXCEPTION.md), which is the authoritative
version of this paragraph:

> As an additional permission under section 7 of the GNU General Public License,
> version 3 (or, at your option, any later version), the copyright holders of
> Rustamp give you permission to combine the covered work with independent
> modules that communicate with the covered work solely through (a) the
> documented Rustamp classic-plugin ABI (the Winamp 2.x `in_`/`out_`/`vis_`/
> `dsp_`/`gen_` plugin interface, as defined by the `rustamp-plugin-abi` crate)
> and (b) the documented Rustamp IPC protocol (the `rustamp-ipc` crate,
> protocol version 1), regardless of the license terms of those independent
> modules, and to copy and distribute the resulting combination under the terms
> of the GNU GPL for the covered work and the licenses of the other code
> concerned, provided that you do not modify those documented interfaces. This
> exception does not apply to material in the covered work that is licensed
> under the GPL by third parties; as to that material, the GPL applies without
> this exception.

The §7 exception is a fallback for the player ↔ plugin-host edge (where the FSF
FAQ's sentence "Using shared memory to communicate with complex data
structures is pretty much equivalent to dynamic linking" is the honest weak
point). It is **not** the instrument that makes your plugin safe: the permissive
host licence in §2 is (RFC 0004 §2).

## 7. GPL decoders and DSP stages as separate helper processes

If you maintain a GPL decoder or DSP implementation, RFC 0006 (accepted
2026-10-15) plans to run such code in its own GPL-3.0-or-later helper processes
(binaries `rustamp-dechost`, `rustamp-dsphost`, in a new crate
`rustamp-mediahost`, both 64-bit) — deliberately **without** the §7 exception, because the
exception cannot cover third-party GPL material — and to keep the player
talking to them over the same documented, arm's-length protocol plus shared
memory as the plugin host. That is the pattern for *compiled-in* copyleft audio
code; it is separate from the classic plugin path, where your `dsp_` DLL still
runs in `rustamp-plughost32.exe` exactly as in §2.

**[provisional]** Whether any *plugin-side* decoder or DSP is ever offered this
route is not decided: RFC 0006 §2 puts bridging classic 32-bit `dsp_` DLLs out
of `plughost32` explicitly out of scope, and RFC 0006 §6 keeps classic `dsp_`
DLLs in `plughost32`. Do not read §7 as a change to how your plugin is hosted.

## 8. What this document is not

It is not API documentation. It does not describe the struct layouts, export
names, call order or lifetimes a plugin must follow — those belong in a
separate, future **plugin-author API guide** (working name:
`docs/plugin-api.md`), written from the project's own specs (S09–S11) once they
are exported for authors. Until it exists, the ABI is described by the
`rustamp-plugin-abi` crate itself. Name this as a follow-up; do not treat this
document as that guide.

## 9. Open questions and provisional points

1. **[provisional] The Ardour pattern is second-hand here.** The notes'
   summary of Ardour is a quoted "Plugin Clarification" in Ardour's `COPYING`:
   "we consider it self-evident that there can be no 'derivative' relationship
   (in the sense of the GPL's use of this term) between the host and plugin"
   and "Ardour may load arbitrarily licensed plugins"
   (`docs/notes/L-licensing-plugins.md` §2.2). The notes do not record *how*
   such a clarification is normally published (a `COPYING` addendum? a website
   page? a `LICENSE` section?) or whether the pattern needs a per-file notice.
   Rustamp has not yet decided the placement of a comparable, plugin-author-
   facing clarification. **Open question:** should Rustamp publish its own
   "plugin clarification" text, and where (this document, `PLUGIN-EXCEPTION.md`,
   the README, or a repo-root file)?
2. **[provisional] The host surface a plugin may rely on is described as a
   design, not as a finished compatibility contract.** `docs/notes/
   L-licensing-plugins.md` §1.6 records the intended fake window, fake
   out-module, embed frames and classic IPC messages; the extent of the
   emulated surface is defined by specs S09–S11 and is still being built. No
   per-plugin compatibility list is published.
3. **No plugin-developer agreement was found, but absence is not proof.** The
   SDK note searched the shipped 5.9 SDK installer, the source release and the
   archived NSDN pages and found no SDK EULA and no click-through terms; it
   records that a click-through agreement could have existed at some earlier
   point (`docs/notes/SDK-licensing.md` Q4, Q4.1).
4. **The classic mini-SDK packages were not inspected**, so whether they
   contained a readme or licence file is unverified
   (`docs/notes/SDK-licensing.md` Q5.1).
5. **Trademark and the compatibility name** — whether a third-party host's use
   of the compatibility name is covered by the WCL's "describing the origin"
   exception or by fair usage — is explicitly unsettled in the notes
   (`docs/notes/SDK-licensing.md` Q5.3).
6. **The §7 exception's behaviour under an "or later" election** to a future
   GPL version is not addressed in the FSF FAQ; the notes mark it
   "[Open question — for a lawyer]" and mitigate it by wording the permission
   to apply under any version the recipient chooses
   (`docs/notes/L-licensing-plugins.md` §3.2, §5 open question 6). The wording
   used is the one quoted in §6.
7. **The plugin-licence census is incomplete.** Individual terms of classic
   binaries, and how many plugins are GPL-2.0-only, are unverified
   (`docs/notes/L-licensing-plugins.md` §5 open questions 2–3).
8. **Support policy is not written down.** Beyond "plugins are user-installed
   and compatibility is tracked as a test matrix", the project has no published
   statement of what plugin-author support it offers. If you need one, ask the
   project rather than assuming.

## 10. Where to read more

| Document | What it is |
|---|---|
| [PLUGIN-EXCEPTION.md](../../PLUGIN-EXCEPTION.md) | The §7 additional permission, authoritative text |
| [docs/rfc/0004-plugin-licensing.md](../rfc/0004-plugin-licensing.md) | The accepted licence split (with errata) |
| [docs/notes/L-licensing-plugins.md](L-licensing-plugins.md) | The full licensing analysis, options and consequences |
| [docs/notes/SDK-licensing.md](SDK-licensing.md) | What the Winamp SDK's own terms say about a third-party host |
| [README.md](../../README.md) (License) and [THIRD_PARTY.md](../../THIRD_PARTY.md) | What Rustamp is licensed under, and what it contains |
| [docs/rfc/0006-out-of-process-hosting.md](../rfc/0006-out-of-process-hosting.md) | Decoder/DSP helper processes (accepted; implementation later) |
| [CLEANROOM.md](../../CLEANROOM.md) | Binding rules on sources for everyone working on Rustamp |
