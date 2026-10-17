# L-licensing-plugins: running proprietary and GPL-incompatible classic Winamp 2.x plugins

Status: licensing research report, decision-ready. Research and analysis only — this report changes no code, no license file, no crate, no doc other than itself.

**Historical snapshot (2026-10-14), current-state clarification 2026-10-17:**
the proposals and dependency descriptions below precede implementation. The
current permissive set is **five** crates: `rustamp-proto`, `rustamp-plugin-abi`,
`rustamp-ipc`, `rustamp-plughost`, `rustamp-mediahost` (protocol framework only).
RFC 0004 §7 and RFC 0006 §8 supersede older lists. Current manifests and
`THIRD_PARTY.md`, not this research snapshot, define the implemented split;
see `docs/tasks/V3-license-readiness.md` for the enforcement audit.

- Written by: licensing research subagent (spawned session), 2026-10-14.
- Question: can Rustamp (GPL-3.0-or-later) run **both proprietary and open-source classic Winamp 2.x plugins** (`in_`, `out_`, `vis_`, `dsp_`, `gen_`)?
- Repo facts used: `docs/PLAN.md` (§1 source table, §2 architecture, milestones), `CLEANROOM.md` (§1, §4, §7), `THIRD_PARTY.md`, `docs/HANDOFF.md`, `LICENSE`, `Cargo.toml` (workspace + crates), `CONTRIBUTING.md`, git metadata (18 commits, one identity, no remote).
- All legal and license claims are cited to a URL (Appendix A). Quotes are verbatim from the fetched pages.

## Evidence labels (used throughout)

| Label | Meaning |
|---|---|
| **[License text]** | The quoted terms of a license (GPL-3.0, LGPL, MIT, Apache-2.0, …). |
| **[FSF interpretation]** | The FSF's published FAQ / license-list positions. Advisory: the FSF is a party and an advocate, not a court. |
| **[Tolerated practice]** | What major projects actually do and the community accepts. **Never court-tested.** |
| **[Court-tested]** | Actual case law or statute, with citation. |
| **[Open question]** | Could not be established online, or must not be researched in this room (see §5). |

---

## 0. Executive summary

**1. Is GPL-3.0 actually a problem here, and why?** Partly — but not in the shape the owner fears, and not where the owner fears it.

- The GPL restricts **conveying** (distributing) works, not **running** them. GPL-3.0 §0: "To 'propagate' a work means to do anything with it that, without permission, would make you directly or secondarily liable for infringement under applicable copyright law, **except executing it on a computer or modifying a private copy**." §2: "This License explicitly affirms your **unlimited permission to run** the unmodified Program." [License text] https://www.gnu.org/licenses/gpl-3.0.html — An end user who runs a GPL player with a proprietary plugin violates nothing. The question is what **Rustamp may distribute**, and what **plugin authors may be assured of**.
- The program that loads plugin DLLs is **not the GPL player** — it is the separate 32-bit process `rustamp-plughost32.exe` (`docs/PLAN.md` §2). The player never loads a plugin DLL into its address space. Under the FSF's own framework, the license that matters for the plugin boundary is **the host's license, not the player's** (§1.6 below).
- If the host is GPL-3.0 and loads plugin DLLs **in-process** (dynamic linking, function calls both ways, shared structs), the FSF's stated position is that host and plugin "form a single combined program, which must be treated as an extension of both" — so a proprietary plugin could not lawfully be distributed together with the host, and the FSF would say the plugin "must be licensed under the GPL or a GPL-compatible free software license" [FSF interpretation] https://www.gnu.org/licenses/gpl-faq.en.html#GPLPlugins and #GPLAndPlugins. That is the real problem — and it is a problem of the **host's** license.
- The separate-process architecture is the fix, and the FSF's own FAQ describes exactly this: "pipes, sockets and command-line arguments are communication mechanisms normally used between two separate programs. So when they are used for communication, the modules normally are separate programs." [FSF interpretation] https://www.gnu.org/licenses/gpl-faq.en.html — **but** the same answer adds: "**Using shared memory to communicate with complex data structures is pretty much equivalent to dynamic linking.**" That sentence is the single strongest textual hook against Rustamp's player↔host shm rings, and this report takes it seriously (§1.4).
- **The reverse-direction problem (the thing the owner probably does not know yet):** GPL-3.0 is also **incompatible with GPL-2.0-only** plugins. FSF FAQ: "Is GPLv3 compatible with GPLv2? **No.** … if you tried to combine code released under GPLv2 with code under GPLv3, you would violate section 6 of GPLv2." [FSF interpretation] https://www.gnu.org/licenses/gpl-faq.en.html#v2v3Compatibility — So even "all open-source plugins, no proprietary ones" is **not** achievable by a GPL-3.0 host that loads plugins in-process: any GPL-2.0-only plugin would conflict with it. The separate-process architecture solves **both** directions at once: proprietary plugins and GPL-2.0-only plugins both run, because no combination is ever formed.

**2. What are the options?** Six options are analyzed in §3: (1) stay GPL-3.0 and rely on the separate-process architecture; (2) stay GPL-3.0 + a GPL-3.0 §7 "additional permission" (plugin exception); (3) relicense the plugin-facing crates (`rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`) to a permissive license while the player stays GPL-3.0; (4) relicense everything to LGPL-3.0; (5) relicense everything to MIT or Apache-2.0; (6) dual licensing / per-file split.

**3. Can the plugin runner be licensed MIT while the rest stays GPL-3.0?** **Yes** — with one mechanical precondition. A separate program may have a different license; the GPL does not force one license on separate programs, and shipping a GPL player and an MIT host in one installer is a "mere aggregation" (GPL-3.0 §5). The precondition: **the MIT host binary must not link GPL code.** Today `rustamp-plughost` and `rustamp-ipc` depend on the GPL `rustamp-core` crate (`crates/rustamp-plughost/Cargo.toml`, `crates/rustamp-ipc/Cargo.toml`; `rustamp-ipc/src/message.rs` and `src/shm/header.rs` use `rustamp_core::{command::Command, state::PlayerState, vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame}}`), so the wire types must first be extracted into a new permissive crate that both sides share (§3.3). Today this is cheap: **18 commits, one git identity (`Rustamp <rustamp@project>`), no remote, nothing published, no CLA** — relicensing the project's own code is a decision, not a negotiation. **That window closes at first publication and at the first external contributor.**

**Primary recommendation (§4.1): Option 3 + Option 2 combined** — permissive license (MIT OR Apache-2.0) for the plugin-facing crates, plus a GPL-3.0 §7 plugin-combination additional permission on the GPL player, modeled on the FSF's own "controlled interface" exception text. **Fallback (§4.2): Option 2 alone** — uniform GPL-3.0 plus the §7 plugin exception on the plugin-facing crates and the player, no relicensing. Options 4 and 5 are **not recommended**: they forfeit decoder sources the plan depends on and do not improve the plugin situation. Option 1 alone is the do-nothing baseline and leaves plugin authors without any grant.

---

## 1. The legal question, precisely

### 1.1 What GPL-3.0 restricts, and what it does not

[License text] All quotes from GPL-3.0, https://www.gnu.org/licenses/gpl-3.0.html (identical to the repo's `LICENSE`).

- §0: "To 'modify' a work means to copy from or adapt all or part of the work in a fashion requiring copyright permission… The resulting work is called a 'modified version' of the earlier work or a work **'based on'** the earlier work." / "A 'covered work' means either the unmodified Program or a work based on the Program."
- §0: "To 'propagate' a work means to do anything with it that, without permission, would make you directly or secondarily liable for infringement under applicable copyright law, **except executing it on a computer or modifying a private copy**."
- §0: "To 'convey' a work means any kind of propagation that enables other parties to make or receive copies. Mere interaction with a user through a computer network, with no transfer of a copy, is not conveying."
- §2: "This License explicitly affirms your **unlimited permission to run** the unmodified Program. The output from running a covered work is covered by this License only if the output, given its content, constitutes a covered work."
- §1 (Corresponding Source): includes "…the source code for shared libraries and dynamically linked subprograms that the work is **specifically designed to require, such as by intimate data communication or control flow** between those subprograms and other parts of the work."
- §5(c): "You must license the **entire work, as a whole**, under this License to anyone who comes into possession of a copy."
- §5 (aggregate): "A compilation of a covered work with other **separate and independent works**, which are not by their nature extensions of the covered work, and which are not combined with it such as to form a larger program, in or on a volume of a storage or distribution medium, is called an 'aggregate'… Inclusion of a covered work in an aggregate **does not cause this License to apply to the other parts** of the aggregate."

Precise consequences for Rustamp:

1. **Running is never restricted.** Users may load any plugin into any host. The GPL's conditions attach to *propagating* (distributing) covered works and to *modified versions* / works *based on* the Program.
2. **The obligation is on distribution.** If distributing the host together with a plugin would create a "combined work" / work "based on" the GPL host, §5(c) requires the whole to be under the GPL — which a proprietary plugin's license forbids. That combination could then not be distributed at all.
3. **Shipping GPL and non-GPL side by side is fine** if they remain separate works (aggregate, §5; FSF FAQ #MereAggregation and #GPLCompatInstaller, §1.2).
4. **Data crossing a process boundary is not code.** The PCM/vis frames the player sends and the pixels a plugin renders are output/data, not a derivative of the player's code: FSF FAQ #WhatCaseIsOutputGPL: "The output of a program is not, in general, covered by the copyright on the code of the program. So the license of the code of the program does not apply to the output, whether you pipe it into a file, make a screenshot, screencast, or video." [FSF interpretation] https://www.gnu.org/licenses/gpl-faq.en.html#WhatCaseIsOutputGPL

### 1.2 The FSF's position on plugins, linking and process boundaries

[License text + FSF interpretation] All from the GNU GPL FAQ, https://www.gnu.org/licenses/gpl-faq.en.html (anchors given), and the FSF license list, https://www.gnu.org/licenses/license-list.en.html.

**The core test — FSF FAQ #GPLPlugins, "When is a program and its plug-ins considered a single combined program?":**

> "It depends on how the main program invokes its plug-ins. If the main program uses fork and exec to invoke plug-ins, and they establish intimate communication by sharing complex data structures, or shipping complex data structures back and forth, that can make them one single combined program. A main program that uses simple fork and exec to invoke plug-ins and does not establish intimate communication between them results in the plug-ins being a separate program. If the main program dynamically links plug-ins, and they make function calls to each other and share data structures, we believe they form a single combined program, which must be treated as an extension of both the main program and the plug-ins. If the main program dynamically links plug-ins, but the communication between them is limited to invoking the 'main' function of the plug-in with some options and waiting for it to return, that is a borderline case. **Using shared memory to communicate with complex data structures is pretty much equivalent to dynamic linking.**"

**The general boundary test — FSF FAQ, "Where's the line between two separate programs, and one program with two parts?"** (in the section *Combining work with code released under the GNU licenses*):

> "**This is a legal question, which ultimately judges will decide.** We believe that a proper criterion depends both on the **mechanism of communication** (exec, pipes, rpc, function calls within a shared address space, etc.) and the **semantics of the communication** (what kinds of information are interchanged). If the modules are included in the same executable file, they are definitely combined in one program. If modules are designed to run linked together in a shared address space, that almost surely means combining them into one program. **By contrast, pipes, sockets and command-line arguments are communication mechanisms normally used between two separate programs. So when they are used for communication, the modules normally are separate programs.** But if the semantics of the communication are intimate enough, exchanging complex internal data structures, that too could be a basis to consider the two parts as combined into a larger program."

**What that means for a plugin author — #GPLAndPlugins:** "If the main program and the plugins are a single combined program then this means you must license the plug-in under the GPL or a GPL-compatible free software license and distribute it with source code in a GPL-compliant way. **A main program that is separate from its plug-ins makes no requirements for the plug-ins.**"

**The mirror image — #NFUseGPLPlugins ("Can I release a nonfree program that's designed to load a GPL-covered plug-in?"):** "…if they are separate works then the license of the plug-in makes no requirements about the main program."

**Linking — #GPLStaticVsDynamic:** "Linking a GPL covered work statically or dynamically with other modules is making a combined work based on the GPL covered work. Thus, the terms and conditions of the GNU General Public License cover the whole combination." See also #IfLibraryIsGPL ("Yes, because the program actually links to the library. As such, the terms of the GPL apply to the entire combination.").

**Mere aggregation — #MereAggregation:** "An 'aggregate' consists of a number of separate programs, distributed together on the same CD-ROM or other media. **The GPL permits you to create and distribute an aggregate, even when the licenses of the other software are nonfree or GPL-incompatible.** The only condition is that you cannot release the aggregate under a license that prohibits users from exercising rights that each program's individual license would grant them." See also #WhatIsCompatible: "If you just want to install two separate programs in the same system, it is not necessary that their licenses be compatible, because this does not combine them into a larger work." And #GPLCompatInstaller: "No. The installer and the files it installs are separate works. As a result, the terms of the GPL do not apply to the installation software."

**System libraries — #SystemLibraryException:** "Both versions of the GPL have an exception to their copyleft, commonly called the system library exception…" (GPL-3.0 §1 defines "System Libraries" and excludes them from Corresponding Source.)

**Who may grant an exception — #GPLIncompatibleLibs:** "Only the copyright holders for the program can legally release their software under these terms. If you wrote the whole program yourself, then assuming your employer or school does not claim the copyright, you are the copyright holder--so you can authorize the exception. **But if you want to use parts of other GPL-covered programs by other authors in your code, you cannot authorize the exception for them. You have to get the approval of the copyright holders of those programs.**"

**"Arms length" — #GPLInProprietarySystem:** "However, in many cases you can distribute the GPL-covered software alongside your proprietary system. To do this validly, you must make sure that the free and nonfree programs **communicate at arms length**, that they are not combined in a way that would make them effectively a single program."

**The FSF's own recommended exception text — #LinkingOverControlledInterface ("How can I allow linking of proprietary modules with my GPL-covered library under a controlled interface only?"):**

> "Add this text to the license notice of each file in the package…: 'Linking ABC statically or dynamically with other modules is making a combined work based on ABC. Thus, the terms and conditions of the GNU General Public License cover the whole combination. **As a special exception, the copyright holders of ABC give you permission to combine ABC program with free software programs or libraries that are released under the GNU LGPL and with independent modules that communicate with ABC solely through the ABCDEF interface.** You may copy and distribute such a system following the terms of the GNU GPL for ABC and the licenses of the other code concerned, provided that you include the source code of that other code when and as the GNU GPL requires distribution of source code **and provided that you do not modify the ABCDEF interface**. Note that people who make modified versions of ABC are not obligated to grant this special exception for their modified versions; it is their choice whether to do so. … **This exception is an additional permission under section 7 of the GNU General Public License, version 3 ('GPLv3')** … Only the copyright holders for the program can legally authorize this exception.'"

**Module licensing — #GPLModuleLicense:** "The GPL says that the whole combined program has to be released under the GPL. So your module has to be available for use under the GPL. **But you can give additional permission for the use of your code.**"

**Interpreters — #InterpreterIncompat:** "When the interpreter just interprets a language, the answer is yes. The interpreted program, to the interpreter, is just data; the GPL doesn't restrict what tools you process the program with." (Relevant to Blender, §2.4.)

**Enforcement — #WhoHasThePower:** "Since the GPL is a copyright license, it can be enforced by the copyright holders of the software."

**The FSF's stance on exceptions for proprietary users — #WillYouMakeAnException:** "Sorry, we don't make such exceptions. It would not be right. Maximizing the number of users is not our aim." (This is about the FSF's own code; it says nothing about what *your* project may do with *its own* code.)

**GPLv2 vs GPLv3 — #v2v3Compatibility:** "No. Many requirements have changed from GPLv2 to GPLv3… the two licenses are not compatible: if you tried to combine code released under GPLv2 with code under GPLv3, you would violate section 6 of GPLv2. **However, if code is released under GPL 'version 2 or later,' that is compatible with GPLv3** because GPLv3 is one of the options it permits." License-list entries agree: GPLv2 "is, by itself, not compatible with GPLv3"; GPLv3 "is not compatible with GPLv2 by itself"; LGPLv2.1 "is compatible with GPLv2 and GPLv3"; LGPLv3 "is compatible with GPLv3… not compatible with GPLv2 by itself"; Apache-2.0 "is compatible with version 3 of the GNU GPL… not compatible with GPL version 2"; Expat (MIT) and zlib "are compatible with the GNU GPL."

**The FSF's own disclaimer (top of the FAQ):** "You can use our publications to understand how GNU licenses work or help you advocate for free software, **but they are not legal advice. The FSF cannot give legal advice.**"

### 1.3 FSF opinion vs. court-tested law

**Court-tested, and relevant:**

- **License conditions are enforceable as copyright conditions.** *Jacobsen v. Katzer*, 535 F.3d 1373 (Fed. Cir. 2008): the Federal Circuit held that open-source license conditions are enforceable under copyright law and that breaching them can constitute copyright infringement. https://jolt.law.harvard.edu/digest/jacobsen-v-katzer ; opinion text: https://www.law.berkeley.edu/archive/files/Jacobsen_v_Katzer_-_Westlaw.pdf — This is why license terms matter at all: a GPL violation is a copyright claim, not a contract quibble.
- **Facts are not copyrightable.** *Feist Publications, Inc. v. Rural Telephone Service Co.*, 499 U.S. 340 (1991). https://supreme.justia.com/cases/federal/us/499/340 — The ABI *facts* (sizes, offsets, constants, call order) that `rustamp-plugin-abi` is written from cannot be protected.
- **Derivative-work / substantial-similarity framework.** *Computer Associates International, Inc. v. Altai, Inc.*, 982 F.2d 693 (2d Cir. 1992) (abstraction–filtration–comparison; elements dictated by function are filtered out). https://law.justia.com/cases/federal/appellate-courts/F2/982/693/137252/ — A struct layout dictated by a C ABI is the kind of expression merger/filtration removes.
- **Interface copyrightability (US) is unsettled at the top.** *Google LLC v. Oracle America, Inc.*, 593 U.S. 1 (2021): the Supreme Court **assumed without deciding** that API "declaring code" is copyrightable and held Google's copying a fair use; the Federal Circuit (2018) had held it copyrightable. https://www.supremecourt.gov/opinions/20pdf/18-956_d18f.pdf ; https://www.law.cornell.edu/supremecourt/text/18-956
- **Interface copyrightability (EU) is settled the other way.** *SAS Institute Inc v World Programming Ltd*, CJEU, Case C-406/10, ECLI:EU:C:2012:259 (Grand Chamber, 2 May 2012): "Neither the functionality of a computer program nor the programming language and the format of data files in order to exploit its functions constitute a form of expression of that program and are not protected by copyright", and "Licensed software users may observe, study, or test the functioning of the program to determine the ideas and principles which underlie its elements…". https://en.wikipedia.org/wiki/SAS_Institute_Inc_v_World_Programming_Ltd ; case page: http://curia.europa.eu/juris/liste.jsf?num=C-406/10 — Directive 2009/24/EC (Software Directive), Art. 1(2) and recital 11: "only the expression of a computer program is protected and … ideas and principles which underlie any element of a program, **including those which underlie its interfaces**, are not protected by copyright under this Directive." https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32009L0024
- **Reverse engineering for interoperability.** US: 17 U.S.C. §1201(f) https://www.law.cornell.edu/uscode/text/17/1201 ; *Sega Enterprises Ltd. v. Accolade, Inc.*, 977 F.2d 1510 (9th Cir. 1992) (disassembly to study unprotected ideas/functional requirements is fair use) https://www.copyright.gov/fair-use/summaries/segaenters-accolade-9thcir1992.pdf ; *Sony Computer Entertainment, Inc. v. Connectix, Inc.*, 203 F.3d 596 (9th Cir. 2000) https://law.justia.com/cases/federal/appellate-courts/F3/203/596/474793/ . EU: Software Directive Art. 5(3) and Art. 6 (decompilation to obtain interoperability information is permitted) — same EUR-Lex link as above; confirmed by SAS v. WPL.

**Not court-tested — say so explicitly:**

- **No reported court decision, in the US or the EU, addresses whether a plugin loaded at runtime is a derivative work of its host, or where the boundary between "separate programs" and "one program" sits for plugins/processes.** The FSF says so itself: "This is a legal question, which ultimately judges will decide." [FSF interpretation] The plugin/linking positions in §1.2 are the FSF's interpretation of copyright law's derivative-work concept, backed by decades of community practice — not holdings.
- GPL enforcement actions have overwhelmingly ended in settlement or voluntary compliance; none tested the plugin boundary. Enforcement is by copyright holders (#WhoHasThePower) — for Rustamp: the project's contributors; for a plugin: the plugin's author.
- The "combined work vs. aggregate" and "intimate communication" tests are therefore **FSF interpretation plus tolerated practice**, and this report labels them as such wherever they are load-bearing.

### 1.4 The "designed to evade the GPL" question

Honest assessment. A reviewer (or a court) could ask: *"Did Rustamp split the process and permissively license the loader specifically so that proprietary plugins escape the GPL?"* Points on each side:

**Against Rustamp (the risk):**

1. The FSF's hook: "Using shared memory to communicate with complex data structures is pretty much equivalent to dynamic linking." [FSF interpretation] Rustamp's player↔host transport is shm rings (`docs/PLAN.md` §2; RFC 0003) carrying `VisFrame` (2×576 samples + spectrum) and PCM — i.e., shared memory with structured data. If player+host were treated as one program, the GPL player's copyleft would reach the host, and through it the plugins.
2. The FSF's general posture is skeptical of arrangements whose *purpose* is to keep nonfree code outside the GPL's reach (#GPLInProprietarySystem: programs must "communicate at arms length… not… effectively a single program"; #WillYouMakeAnException shows the FSF's refusal to help proprietary users of *its* code).
3. The FSF has published **no** specific "anti-evasion" doctrine for process-split architectures. There is no case law. The risk is therefore mostly **reputational and interpretive**, not a clear rule.

**For Rustamp (the mitigations):**

1. **The process split is technically forced, not chosen for licensing.** Microsoft: "On 64-bit Windows, a 64-bit process cannot load a 32-bit dynamic-link library (DLL)." https://learn.microsoft.com/en-us/windows/win32/winprog64/process-interoperability — Classic plugins are 32-bit x86 DLLs; the player is 64-bit. They *cannot* share an address space; they *must* be separate executables. The plan's crash-isolation requirement ("restart the host on plugin crash", `docs/PLAN.md` M3) is a second, independent, non-legal reason for the split. A court or reviewer asks *why* two pieces are one work; here the answer is bitness, not license strategy.
2. **The communication is a documented, versioned wire protocol, not shared internal state.** The control channel is a named pipe; the data channel is a lock-free ring with a published, byte-identical layout, length framing, a 16 MiB cap and generation guards (`rustamp-ipc`, protocol v1, RFC 0003). The FSF's "intimate" test is about "exchanging complex *internal* data structures" — internal state of one program. A versioned wire protocol with a public spec is an interface. (Still [FSF interpretation]; the shm sentence remains the honest weak point, which is why the recommendation adds a §7 permission on the player, §4.1.)
3. **GIMP does exactly this, under the GPL, and the FSF has never objected.** GIMP is GPL-3.0-or-later and runs plug-ins as "independent processes started by the core GIMP process… exchanging information through Inter-Process Communication (the 'wire')" (official GIMP docs, §2.1). The FSF's fork-and-exec/pipes language in #GPLPlugins *is* the GIMP model.
4. **Ardour goes further and is tolerated.** Ardour is GPL-2.0-or-later and loads arbitrarily-licensed VST plugins **in-process**, with a published "Plugin Clarification" asserting there is no derivative relationship because the interaction is defined by a third-party API (§2.2). Rustamp's design is strictly more conservative than Ardour's on the plugin edge: the process that loads the DLLs will not be GPL-licensed at all.
5. **The player stays GPL.** The copyleft core (the whole player, all its crates) is not evaded; only the thin, ABI-defined plugin-loading shim becomes permissive. The FSF's own blessed mechanism for "I want proprietary modules to interoperate with my GPL program" is a controlled-interface exception (#LinkingOverControlledInterface) — i.e., the FSF's practical answer to this goal is *an explicit, documented permission*, which is exactly what Option 2 provides and what Option 3 makes structural.
6. **Transparency.** The decision is made openly, in an RFC, with the reasoning on record — the opposite of concealment.

**Residual risk, stated plainly:** if the FSF (or a court) applied the shm sentence literally to player↔host and concluded "one program", the GPL player's copyleft would reach the host. For Rustamp's own code that is cured by the §7 additional permission (Option 2). It is **not** curable by us for third-party GPL code inside the player (e.g. sidplayfp via FFI — see §3.7): under that strictest theory, the tail risk is the decoder libraries, not the plugin host. Mitigations: the architecture (the host is a separate program and will be permissively licensed), and, if the owner ever wants zero residual risk, moving GPL-2.0+ decoder code into a separate decoder-host process — the same pattern as the plugin host (§3.7, §4.3).

### 1.5 The reverse-direction problem: GPL-2.0-only and other GPL-incompatible plugins

- **GPL-2.0-only is incompatible with GPL-3.0.** [FSF interpretation] FSF FAQ #v2v3Compatibility: "No… if you tried to combine code released under GPLv2 with code under GPLv3, you would violate section 6 of GPLv2." License-list, GPLv2 entry: "GPLv2 is, by itself, not compatible with GPLv3." https://www.gnu.org/licenses/gpl-faq.en.html#v2v3Compatibility ; https://www.gnu.org/licenses/license-list.en.html#GPLv2
- **Other GPL-incompatible free licenses exist** and plugins may carry them: the FSF's "GPL-Incompatible Free Software Licenses" list includes (among others) AGPLv1, the Academic Free License, Apache-1.0/1.1, APSL-2, and more. [FSF interpretation] https://www.gnu.org/licenses/license-list.en.html#GPLIncompatibleLicenses
- **Plugin licenses in the wild span every class.** Verified examples: proprietary/closed (Nullsoft's bundled `in_mp3.dll`, `out_ds.dll`, … are closed-source freeware binaries shipped inside the Winamp installer — see §5, open question 2; Winamp 5.x itself was released in 2024 under a license that prohibits forking and distribution, https://hackaday.com/2024/09/24/winamp-releases-source-code-but-is-it-really-open/ ; WACUP is closed source per `CLEANROOM.md` §1); permissive (AVS: "made open source… in May 2005, released under a BSD-style license" https://sr.ht/~grandchild/AVS/ and https://getmusicbee.com/addons/visualizer/367/advanced-visualization-studio/ ; `in_openmpt` is part of the libopenmpt project, BSD-3-Clause, https://lib.openmpt.org/libopenmpt/license/ ; MilkDrop 2.25c `vis_milk2/` source: Nullsoft BSD-3-Clause — verified in the zip, §3.7); copyleft (various; some plugins are GPL-2.0-only — **open question**, §5).
- **Consequence:** a GPL-3.0 host that loads plugins in-process cannot lawfully be distributed together with *any* plugin whose license is not GPL-3.0-compatible — neither proprietary nor GPL-2.0-only nor AFL/CDDL-style. "Support all open-source plugins" is therefore **not** achievable under GPL-3.0 in-process, even with zero proprietary plugins. Only the separate-process architecture (host not GPL, or GPL + plugin exception) supports the full spectrum.
- Nuance: an unversioned "GPL" notice on a plugin is ambiguous (GPL-1.0+? GPL-2.0+?) — check per plugin. **[Open question]**

### 1.6 Where the boundary sits for Rustamp

Facts from `docs/PLAN.md` §2 and `docs/HANDOFF.md`:

- `rustamp.exe` (x64, GPL-3.0-or-later) never loads a plugin DLL. It spawns `rustamp-plughost32.exe` (i686) and talks to it over shm rings + a control pipe.
- `rustamp-plughost32.exe` loads `in_`/`out_`/`vis_`/`dsp_`/`gen_` DLLs **into its own address space** and emulates a fake "Winamp v1.x" main window (`WM_WA_IPC`), a fake out-module, and embed frames. Plugin↔host interaction is classic in-process dynamic linking: the host calls plugin exports, plugins call host callbacks, both share `#[repr(C)]` structs (`rustamp-plugin-abi`, per spec S09).
- The host is crash-isolated (restart on plugin crash) and independently versioned; the two sides are different bitness and cannot share an address space (§1.4).

Applying §1.2:

1. **Plugin ↔ host edge (in-process, intimate):** under the FSF's test this is "dynamic linking + function calls + shared data structures" → "single combined program". **The license that governs this edge is the host's license.** If the host is GPL-3.0, the FSF's stated position is that any plugin loaded by it must be GPL-compatible — proprietary and GPL-2.0-only plugins could not be distributed with it (§1.5). If the host is permissively licensed (MIT/Apache-2.0), no copyleft reaches the plugin at all, and the edge is clean by construction.
2. **Player ↔ host edge (separate processes, pipe + shm):** the FSF's framework treats separate executables communicating over pipes/sockets as separate programs; the shm-with-structured-data sentence is the honest counter-hook (§1.4). Both sides are Rustamp's own code, so this edge is about whether the GPL player's copyleft reaches the host — curable for our code by a §7 additional permission (Option 2), and moot for the host's own license if the host is MIT.
3. **Player ↔ plugin:** no interaction at all (the player never touches plugin DLLs, and the data crossing the boundary is output/data, §1.1(4)). The GPL player imposes nothing on plugins.
4. **Distribution model:** Rustamp does not ship plugins; users install them. Shipping the GPL player + permissive host in one installer is an aggregate (GPL-3.0 §5; FSF FAQ #MereAggregation, #GPLCompatInstaller). Bundling any *specific* plugin is a separate, per-plugin license decision (§3.8).

**Bottom line for §1:** the owner's concern is real but mis-aimed. GPL-3.0 on the *player* is not what blocks mixed-license plugins; GPL-3.0 on the *host* is. The architecture already isolates the player; the license decision that matters is the host's.

---

## 2. Precedents

### 2.0 Overview table

| Project | License (verified) | Plugin mechanism | Position on plugin licenses | Court-tested? |
|---|---|---|---|---|
| GIMP | GPL-3.0-or-later | Plug-ins are **separate processes** over a pipe ("the wire") | Plug-ins are separate programs; proprietary plug-ins exist | No — tolerated practice |
| Ardour | GPL-2.0-or-later | VST/AU/LV2/LADSPA/CLAP loaded **in-process** (dynamic loading) | Published "Plugin Clarification": host and plugin have no derivative relationship because the interaction is defined by a 3rd-party API; "Ardour may load arbitrarily licensed plugins" | No — tolerated practice |
| Audacity | GPL-3.0 (project level; many files GPL-2.0-or-later) | VST effects loaded in-process; user drops DLLs into the Plug-Ins folder | Ships VST support; SDK-license saga resolved by Steinberg going MIT (Oct 2025) | No — tolerated practice |
| Blender | GPL-2.0-or-later | Python add-ons run **in-process** in the interpreter | **Strict view**: "such scripts (if published) are being shared under a GPL compliant license" | No — project policy |
| WordPress | GPL-2.0-or-later | PHP plugins/themes loaded in-process | **Strict view**: "Derivatives of WordPress code inherit the GPL license" | No — project policy |
| LMMS | GPL-2.0-or-later | VST instruments loaded in-process via the built-in VeSTige host | VST support shipped; no formal legal statement found | No — tolerated practice |
| VLC | GPL-2.0-or-later | Modules loaded in-process (dlopen) | All official modules GPL-compatible; libVLC relicensed to LGPL (2011) so others may build on it | No — tolerated practice |
| Wine | LGPL-2.1-or-later | Runs proprietary Windows programs (OS reimplementation) | Proprietary apps run on Wine by design | No — tolerated practice |
| OBS Studio | GPL-2.0 | Plugins loaded in-process | "OBS software license does not prevent you from monetizing plugins. Just don't bundle OBS Studio with your plugin" (project moderator) | No — tolerated practice |
| DeaDBeeF | core: zlib; plugins: zlib / GPL-2.0-or-later / LGPL-2.1-or-later per plugin | Plug-ins loaded in-process | **Plugin API header (`deadbeef.h`) is zlib-licensed** so that plugins of any license can be written | No — project design |
| Mozilla Firefox | MPL-2.0 | Proprietary plugins/CDMs; media plugins in separate sandboxed processes | MPL-2.0 lets you distribute executables "under a license of your choosing" as long as MPL source rights are preserved ("Larger Work") | No — license design |
| VirtualBox | Base package GPL-3.0; Extension Pack proprietary (PUEL), separate download | — | GPL core + separately-licensed proprietary add-on pack, same vendor | No — business model |

### 2.1 GIMP — the strongest precedent for the architecture

- License: **GPL-3.0-or-later** (COPYING is GPLv3; source headers: "either version 3 of the License, or (at your option) any later version"). https://gitlab.gnome.org/GNOME/gimp/-/raw/master/COPYING ; e.g. https://gitlab.gnome.org/GNOME/gimp/-/raw/master/app/core/gimp.c ; distro metadata: https://packages.fedoraproject.org/pkgs/gimp3/gimp3/
- Mechanism: plug-ins are separate executables. GIMP User Manual: "**Because plugins are separate programs**, they communicate with GIMP in a special way: The GIMP developers call it 'talking over a wire'. When a plugin crashes, the communication breaks down, and you may see an error message about a 'wire read error'." https://docs.gimp.org/2.10/en/gimp-scripting.html — GIMP developer docs: "Plug-ins are the historical way of extending GIMP. They are **independent processes started by the core GIMP process** (so with the same rights as your GIMP process) and exchanging information through Inter-Process Communication (the 'wire'). This is a very robust architecture because even if a plug-in were to crash, the core process (where the image is) would not budge." https://developer.gimp.org/resource/writing-a-plug-in/tutorial-c-basic/
- Position: GIMP is GPL-3.0-or-later and its plug-in ecosystem includes proprietary plug-ins; the FSF's #GPLPlugins fork-and-exec/pipes language describes exactly this architecture, and the FSF has never objected to GIMP's plug-in model. [Tolerated practice]
- Why it matters for Rustamp: GIMP shows that a GPL program may deliberately run plug-ins as separate processes over a pipe, and that this is accepted as keeping plug-ins separate programs. Rustamp's player↔host edge is the same pattern (with shm instead of a pipe — see §1.4 for the honest caveat).

### 2.2 Ardour — the strongest precedent for the plugin edge, and the model for a written clarification

- License: **GPL-2.0-or-later** (its COPYING states: "the relationship between Ardour's license (GPL v2.0 or later) and plugins"). https://raw.githubusercontent.com/Ardour/ardour/master/COPYING
- Mechanism: VST2/VST3/AU/LV2/LADSPA/CLAP plugins are **dynamically loaded in-process** by a GPL-2.0-or-later DAW — a strictly more aggressive configuration than Rustamp's.
- Position — Ardour's COPYING contains a published **"Plugin Clarification"**:

> "The following states the Ardour developers understanding of the relationship between Ardour's license (GPL v2.0 or later) and plugins. For the purposes of this understanding, 'plugins' refer to software dynamically loaded by another program, where the interaction between the plugin and the loading program (typically called 'the host') is defined by a **3rd party API**. Such APIs include (but are not limited to) VST, AudioUnits, LV2, LADSPA and CLAP. Since (a) any such plugin may be loaded by any host that supports the API and (b) any host may load any plugin that supports the API, **we consider it self-evident that there can be no 'derivative' relationship (in the sense of the GPL's use of this term) between the host and plugin.** Consequently, **Ardour may load arbitrarily licensed plugins**, so as to cause interactions between Ardour and the plugin via a 3rd party API, without any implications for either Ardour or the plugin. In our understanding, using such plugins with Ardour is not impacted in any way by Ardour's GPL licensing."

- Why it matters for Rustamp: (a) the Winamp 2.x plugin ABI is even more "3rd-party" than VST — it is a published ABI implemented by many independent hosts (Winamp 2.95/5.x, WACUP, and other Winamp-compatible players), so Ardour's "any host may load any plugin that supports the API" argument applies *a fortiori*; (b) Ardour shows the community practice of publishing a written clarification; (c) if in-process loading under GPL-2.0-or-later with a published clarification is tolerated, then loading through a separate permissively-licensed process is strictly safer. [Tolerated practice — never court-tested.]

### 2.3 Audacity — GPL audio editor loading proprietary VST plugins in-process

- License: **GPL-3.0 at project level** ("Audacity is released under the GNU General Public License version 3 (GPLv3). Individual source files may be available under other licenses… many source files are available under GPL version 2 (GPLv2) or (at your option) any later version…"). https://raw.githubusercontent.com/audacity/audacity/master/LICENSE.txt ; https://www.audacityteam.org/about/license/
- Mechanism: "To add a new VST effect most easily on Windows, put its DLL file into the 'Plug-Ins' folder where you have Audacity installed… Then use Effect > Plugin Manager to enable the VST effects and **load them into Audacity**." https://manual.audacityteam.org/man/effect_menu_vst.html — in-process loading of user-installed, often proprietary, VST DLLs.
- The SDK-license saga: Audacity's VST support historically depended on Steinberg's proprietary VST2 SDK license; Steinberg's October 2025 announcement relicensed the **VST3 SDK (3.8.0) under the MIT license** and made ASIO dual-licensed (proprietary + open-source/GPL-compatible option). https://www.steinberg.net/press/2025/vst-3-8/ — Lesson for Rustamp: the *SDK's* license (can we even build against it?) is a separate question from the *GPL boundary* (does the GPL reach the plugin?) — see open question 1 (§5).
- [Tolerated practice — never court-tested.]

### 2.4 Blender and WordPress — the strict side of the spectrum

- Blender: **GPL-2.0-or-later** ("The source code we develop at blender.org is default being licensed as GNU GPL Version 2 or later"). https://www.blender.org/about/license/ — Python add-ons run in-process in Blender's embedded interpreter, and Blender's official position is the strict one: "Add-ons (Python scripts): Blender's Python API is an integral part of the software… **The GNU GPL license therefore requires that such scripts (if published) are being shared under a GPL compliant license.**" (Same page.) Blender also demonstrates a **per-module license split inside one GPL project** (Cycles = Apache-2.0, Python = Python License, Bullet = zlib, Libmv = MIT, OSL = BSD — all GPL-compatible).
- WordPress: **GPL-2.0-or-later**; its license page states that plugins and themes are derivative works: "Derivatives of WordPress code inherit the GPL license." https://wordpress.org/about/license/ — PHP plugins are loaded in-process.
- Why they matter: they show the *strict* in-process interpretation exists and is enforced as project policy — which is exactly why the process boundary (GIMP, Rustamp) or a permissive host (Option 3) is the safer design, and why the FSF's mechanism+semantics test (§1.2) is the right frame: Blender/WordPress load plugins *in a shared address space with intimate communication*; Rustamp does not.

### 2.5 LMMS

- License: **GPL-2.0-or-later** (source headers: "either version 2 of the License, or (at your option) any later version"), e.g. https://raw.githubusercontent.com/LMMS/lmms/master/src/core/Engine.cpp
- Mechanism: VST instruments loaded **in-process** through the built-in **VeSTige** host instrument ("VeSTige is a VST host that allow[s] the use of external VST plugins"). https://docs.lmms.io/user-manual/instruments/vestige — historically built against clean-room "vestige"-style VST headers to avoid the Steinberg SDK license.
- [Tolerated practice — never court-tested.] Same lesson as Ardour/Audacity: GPL DAW + in-process proprietary VST plugins is shipped and accepted.

### 2.6 VLC

- License: **GPL-2.0-or-later** ("VLC is released under the GPLv2 (or later) license"). https://github.com/videolan/vlc/blob/master/README.md ; VideoLAN press release: "The license of the VLC media player will continue to be GPLv2 or later." https://images.videolan.org/press/lgpl.html
- Mechanism: modules are loaded in-process (dlopen); all *official* modules are GPL-compatible; binary/proprietary codecs arrive through LGPL libraries (libavcodec), not as VLC modules.
- Relicensing precedent: "VideoLAN and VLC developers have completed the process of changing the license of the VLC engine to LGPL" (libVLC, Dec 2011) — i.e., relicensing a whole engine required **all developers' agreement**. https://images.videolan.org/press/lgpl-libvlc.html — This is the mechanics precedent for Options 4/5 (§3.4): relicensing is a rightsholder decision, and with many contributors it needs a process (CLA or per-contributor consent).
- [Tolerated practice.]

### 2.7 Wine

- License: **LGPL-2.1-or-later** (source headers: "either version 2.1 of the License, or (at your option) any later version"), e.g. https://raw.githubusercontent.com/wine-mirror/wine/master/dlls/kernel32/process.c ; COPYING.LIB: https://gitlab.winehq.org/wine/wine/-/raw/master/COPYING.LIB
- Mechanism: Wine is an OS/API reimplementation that runs **proprietary Windows applications** — the extreme case of "nonfree programs on top of free code", accepted because the Windows API is a published interface and the programs are separate works loaded as data/code by the OS layer.
- Why it matters: (a) precedent that reimplementing a proprietary *interface* and hosting the proprietary programs that use it is accepted practice; (b) precedent that clean-room API reimplementation (Wine's headers) is tolerated (§3.3(c)); (c) LGPL is the license chosen precisely so proprietary software can build on it. [Tolerated practice.]

### 2.8 OBS Studio

- License: **GPL-2.0** (project description: "It's distributed under the GNU General Public License"; forum: "OBS is distributed under the GPLv2 license"). https://github.com/obsproject/obs-studio
- Mechanism: plugins loaded in-process.
- Position (project moderator, official forum): "**OBS software license does not prevent you from monetizing plugins. Just don't bundle OBS Studio with your plugin**, as the GPLv2 has certain restrictions to guarantee user freedom." https://obsproject.com/forum/threads/new-plugin.142141/ — i.e., the project's practical stance: plugins may be proprietary and sold; the constraint is on *bundling*, not on the plugin's license.
- Why it matters: a modern, high-profile GPL-2.0 application whose plugin ecosystem includes commercial proprietary plugins, with the vendor's blessing. [Tolerated practice — never court-tested.]

### 2.9 DeaDBeeF — a GPL-family player that permissively licenses its plugin API

- License: **core is zlib** ("DeaDBeeF core uses ZLIB license. See COPYING in each subdirectory for details." — README §3, https://raw.githubusercontent.com/DeaDBeeF-Player/deadbeef/master/README ); plugins are per-plugin zlib / GPL-2.0-or-later / LGPL-2.1-or-later (verified: `plugins/cdda`, `plugins/ffmpeg` = GPL-2.0-or-later; `plugins/adplug`, `plugins/dumb`, `plugins/gme` = zlib; AUR metadata: "Licenses: zlib, GPL-2.0-or-later, LGPL-2.1-or-later", https://aur.archlinux.org/packages/deadbeef-git).
- **The plugin API header `include/deadbeef/deadbeef.h` is zlib-licensed** ("Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely…"). https://raw.githubusercontent.com/DeaDBeeF-Player/deadbeef/master/include/deadbeef/deadbeef.h
- Why it matters: a player project deliberately permissively licensing its **plugin-facing API header** so that plugins of any license (including proprietary) can be written against it. This is the ecosystem-design precedent for making `rustamp-plugin-abi` permissive (Option 3) — and it is exactly what the PLAN's source table already anticipates ("DeaDBeeF … its zlib core and its GPL/LGPL plugins").

### 2.10 Browsers and VirtualBox (short)

- **Mozilla Firefox (MPL-2.0):** proprietary plugins and content decryption modules (Widevine CDM etc.) run in Firefox; media plugins run in separate sandboxed processes. MPL-2.0 FAQ Q8: "You may distribute any executables you create **under a license of your choosing**, as long as that license does not interfere with the recipients' rights to the source under the terms of the MPL." https://www.mozilla.org/en-US/MPL/2.0/FAQ/ — the "Larger Work" concept: MPL code may be combined with code under other licenses. Chromium (BSD-3) similarly loads the proprietary Widevine CDM.
- **VirtualBox:** base package **GPL-3.0** ("The VirtualBox base package contains the full VirtualBox source code and platform binaries and is licensed under the GNU General Public License, version 3."), with a separately downloaded, separately licensed proprietary **Extension Pack (PUEL)**. https://www.virtualbox.org/wiki/Licensing_FAQ — precedent for "GPL core + proprietary add-on pack distributed by the same vendor, separately licensed".

### 2.11 What has actually been tested in court

**Nothing in §2.1–§2.10 has been tested in court.** The plugin/process boundary question has no reported decision in the US or the EU. What *is* court-tested: license conditions are enforceable as copyright conditions (*Jacobsen v. Katzer*, Fed. Cir. 2008); facts are not copyrightable (*Feist*, 1991); the abstraction–filtration–comparison framework for software similarity (*Altai*, 2d Cir. 1992); US interface copyrightability is unsettled at the Supreme Court level (*Google v. Oracle*, 2021 — fair use decided, copyrightability assumed); EU: functionality and underlying ideas (including interfaces) are not protected, and observation/study/testing and interoperability decompilation are permitted (*SAS v. WPL*, CJEU 2012; Software Directive 2009/24/EC); reverse engineering for interoperability is fair use in the Ninth Circuit (*Sega v. Accolade* 1992; *Sony v. Connectix* 2000) and is carved out by 17 U.S.C. §1201(f). Everything else in this report about the plugin boundary is [FSF interpretation] or [tolerated practice].

---

## 3. The options, with consequences

### 3.0 Comparison table

| # | Option | What changes | Proprietary plugins | GPL-2.0-only plugins | Permissive/GPL-3.0 plugins | Third-party source consequences | Effort | Residual risk |
|---|---|---|---|---|---|---|---|---|
| 1 | Stay GPL-3.0, rely on separate-process architecture | Nothing (document only) | Run (user-installed); no grant to authors; can't bundle with a GPL host | Run; can't be bundled with a GPL-3.0 host (license conflict) | Run | None | Low | No explicit assurance; FSF's combined-work view of a GPL host unaddressed; shm hook unaddressed |
| 2 | GPL-3.0 + §7 plugin-combination additional permission | License notice text on our crates | Run + explicit permission to combine with our code; bundling our host with plugins permitted as to our code | Run; bundling still a license conflict (their GPL-2.0-only vs our GPL-3.0) | Run | None | Low | Permission removable by downstream (GPL §7); doesn't cover third-party GPL code in our crates |
| 3 | Permissive license for plugin-facing crates (player stays GPL-3.0) | Relicense `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` (+ new `rustamp-proto`); decouple from `rustamp-core` | Run, unambiguous: the loading process is not GPL at all | Run, unambiguous | Run, unambiguous | None for these crates (no third-party code in them today) | Medium (one refactor + license files) | "Designed to evade" optics (mitigated, §1.4); needs contributor policy before external PRs |
| 4 | Whole project → LGPL-3.0 | Relicense everything | Same as 3 for the host (if host is LGPL — see §3.4 caveat) | Run | Run | **Forfeits** copied GPL-2.0-or-later code (DeaDBeeF GPL plugins, sidplayfp if copied); in-process FFI to sidplayfp effectively forces GPL-3.0 | High | Doesn't improve the plugin question; LGPL on an app is off-label (FSF "why not LGPL") |
| 5 | Whole project → MIT or Apache-2.0 | Relicense everything | Same as 3 | Run | Run | **Forfeits** copied GPL-2.0+ and LGPL-2.1+ code; in-process FFI to sidplayfp (GPL-2.0+) not permitted from a permissive binary → drop, replace, or separate GPL decoder-host process | High | Loses copyleft entirely; owner's strategic choice |
| 6 | Dual licensing / per-file split | Business model (GPL OR commercial) and/or per-crate split | Unchanged | Unchanged | Unchanged | Requires contribution tracking (CLA) | Medium | Not needed for the plugin goal; per-crate split = Option 3's mechanism |

### 3.1 Option 1 — stay GPL-3.0, rely on the separate-process architecture

What it is: no license change. The player stays GPL-3.0-or-later; the host stays GPL-3.0-or-later; plugins are user-installed and run in the separate 32-bit process.

- **Running:** unrestricted for all plugin classes (GPL-3.0 §0/§2; §1.1). CI's plugin compatibility matrix and end users violate nothing by running plugins.
- **Distributing:** the installer ships the GPL player + GPL host (fine, both GPL). Plugins are not shipped. If Rustamp ever *bundles* a plugin, that plugin's license must be GPL-3.0-compatible (proprietary and GPL-2.0-only plugins: no).
- **The FSF's stated view of the host:** the host loads plugins in-process → under #GPLPlugins, host+plugin "form a single combined program" → a plugin distributed with the host "must be licensed under the GPL or a GPL-compatible free software license" (#GPLAndPlugins). Rustamp doesn't distribute plugins, so this is not a present violation — but it means (a) plugin authors get **no assurance** and may refuse or ask; (b) Rustamp could not ship a curated plugin set, a plugin installer, or "works with MilkDrop/AVS" bundles; (c) the project's plugin support sits on an interpretation the FSF's own FAQ describes as making the plugin part of the host.
- **The shm hook (§1.4)** against player↔host remains unaddressed by any grant.
- Verdict: legally survivable for "run both kinds", but it leaves the goal half-done: no assurance for plugin authors, no bundling, and a documented FSF position against the configuration. **Not recommended as-is; acceptable only as a documented baseline.**

### 3.2 Option 2 — GPL-3.0 + a §7 plugin-combination additional permission

**How §7 works.** [License text] GPL-3.0 §7: "'Additional permissions' are terms that supplement the terms of this License by **making exceptions from one or more of its conditions**. Additional permissions that are applicable to the entire Program shall be treated as though they were included in this License…" A plugin exception makes an exception from §5(c)'s condition that "the entire work, as a whole" be under the GPL. GPL-3.0 has §7; **GPL-2.0 does not** — for GPL-2.0 code, exceptions are added as separate notices (Classpath, MySQL FLOSS). Rustamp is GPL-3.0-or-later, so §7 applies directly. https://www.gnu.org/licenses/gpl-3.0.html

**What such an exception can do:** grant rights over **our own code** — e.g., "you may combine the covered work with independent modules that communicate with it solely through the documented plugin ABI / IPC protocol, regardless of those modules' licenses, and distribute the combination under the GPL for the covered work and the modules' own licenses." The FSF publishes model text for precisely this (#LinkingOverControlledInterface, quoted in §1.2) and calls it "an additional permission under section 7 of the GNU GPL, version 3".

**What it cannot do (be precise):**

1. **It cannot override a plugin author's license.** It grants permissions in *our* code; the plugin's license governs the plugin. (It solves *our* side of the boundary, which is the side we control.)
2. **It cannot cover third-party GPL code inside our crates.** FSF FAQ #GPLIncompatibleLibs: "if you want to use parts of other GPL-covered programs by other authors in your code, **you cannot authorize the exception for them**. You have to get the approval of the copyright holders of those programs." So the exception must be attached per-crate, to crates containing only our code (today: all plugin-facing crates, and `rustamp-core`; **not** `rustamp-decoders` if it carries DeaDBeeF/sidplayfp-derived code).
3. **It does not bind downstream modifiers.** The FSF's model text: "people who make modified versions of ABC are not obligated to grant this special exception for their modified versions; it is their choice whether to do so." And GPL-3.0 §7: "When you convey a copy of a covered work, **you may at your option remove any additional permissions** from that copy, or from any part of it." So a fork or a distro can strip the exception — the assurance is weaker than a license. This is the main reason Option 3 (a real license on the plugin-facing crates) is the primary recommendation.
4. **Interface-tied.** Per the FSF's model text, the exception applies only if the documented interface is not modified ("If you modify the ABCDEF interface, this exception does not apply to your modified version of ABC, and you must remove this exception when you distribute your modified version."). Rustamp's interfaces are versioned (plugin ABI per S09; `rustamp-ipc` protocol v1) — tie the exception to the documented versions.
5. **"Or later" nuance.** The project is GPL-3.0-or-later; whether a §7 permission granted under v3 survives a recipient's election of a hypothetical later GPL version is not addressed in the FSF FAQ. **[Open question — for a lawyer.]** Mitigation: word the exception to apply under any version of the GPL the recipient chooses.

**Real examples of plugin/linking exceptions in the wild:**

- **GNU Classpath exception** (GPL-2.0 + exception): "As a special exception, the copyright holders of this library give you permission to link this library with independent modules to produce an executable, **regardless of the license terms of these independent modules**, and to copy and distribute the resulting executable under terms of your choice… An independent module is a module which is not derived from or based on this library." https://www.gnu.org/software/classpath/license.html
- **GCC Runtime Library Exception 3.1**: "This GCC Runtime Library Exception ('Exception') is an **additional permission under section 7 of the GNU General Public License, version 3** ('GPLv3')." https://www.gnu.org/licenses/gcc-exception-3.1.en.html
- **MySQL FLOSS exception** (GPL-2.0 + exception): "As a special exception to the terms and conditions of version 2.0 of the GPL: 0. You are free to distribute Derivative Works that are formed entirely from the Program and one or more works licensed under one or more of the licenses listed below…" Canonical page: https://www.mysql.com/about/legal/licensing/foss-exception/ ; text quoted at https://lists.debian.org/debian-legal/2004/05/msg00900.html
- **FSF's model controlled-interface exception** (for GPLv3, §7): #LinkingOverControlledInterface, quoted in §1.2. Also: the GPL Font Exception (#FontException) and the Autoconf/Bison exceptions are further §7-style examples. https://www.gnu.org/licenses/gpl-faq.en.html#FontException

**Drafting guidance for Rustamp:** attach the permission per-crate (a `PLUGIN-EXCEPTION.md` + a notice in each crate root) to the crates that contain only Rustamp's own code; tie it to (a) the documented Winamp-2.x plugin ABI (`rustamp-plugin-abi`) and (b) the documented IPC protocol (`rustamp-ipc`, protocol v1); state it as an additional permission under §7 of GPL-3.0 (or later); keep the FSF's "do not modify the interface" condition; provide a suggested text in §4.4. Verdict: **cheap, FSF-blessed, keeps everything GPL — but removable downstream and per-crate-scoped. Best used as the companion to Option 3, or as the fallback on its own.**

### 3.3 Option 3 — permissive license for the plugin-facing crates (the owner's question: can the plugin runner be MIT while the rest stays GPL-3.0?)

**Short answer: yes.** A separate program may have a different license. The GPL does not force one license onto separate programs; shipping a GPL player and an MIT host in one installer is a "mere aggregation" (GPL-3.0 §5; FSF FAQ #MereAggregation, #GPLCompatInstaller). The host is not a work "based on" the GPL player: it implements a documented protocol, and an interface is not a derivative work (§1.3: *Google v. Oracle*, *SAS v. WPL*, merger). The player may keep depending on the permissive `rustamp-ipc` crate (MIT/Apache-2.0 are GPL-compatible, one-way — the permissive code may go into the GPL project, never the reverse).

**The mechanics, in order:**

**(a) Relicensing needs the consent of all copyright holders of the affected code.** Today: **18 commits, one git identity (`Rustamp <rustamp@project>`), no remote, nothing published, no CLA in `CONTRIBUTING.md`** — the project's own rightsholders are a single decision-maker, so relicensing the plugin-facing crates is a decision, not a negotiation. The plugin-facing crates contain **no third-party code today** (`rustamp-plugin-abi` is written from specs; `rustamp-ipc` is original; `rustamp-plughost` is a stub), so no third-party consent is needed. Two caveats: (i) the window closes — once the repo is published under GPL-3.0 and external contributors arrive, relicensing needs their consent (or a CLA); **decide now, and add an inbound=outbound licensing line to `CONTRIBUTING.md` and the PR template before the first external PR** (§4.4); (ii) the webamp MIT code lives in `rustamp-skin` (a GPL crate) — unaffected, and MIT code may stay MIT inside a GPL crate.

**(b) A binary that links GPL crates cannot itself be permissive.** Dynamic linking is a combined work and "the terms and conditions of the GNU General Public License cover the whole combination" (FSF FAQ #GPLStaticVsDynamic). So `rustamp-plughost32.exe` cannot be MIT while it links the GPL `rustamp-core`/`rustamp-ipc` crates. Current dependency evidence: `crates/rustamp-plughost/Cargo.toml` depends on `rustamp-core`, `rustamp-ipc`, `rustamp-plugin-abi`; `crates/rustamp-ipc/Cargo.toml` depends on `rustamp-core`, `rustamp-plugin-abi`; `rustamp-ipc/src/message.rs` uses `rustamp_core::{command::Command, state::PlayerState, vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame}}` and `src/shm/header.rs` uses `rustamp_core::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame}` (transport tests also use `playlist::{EntryId, MediaLocation, SortKey}` and `state::WindowId`). `rustamp-plugin-abi` has **no internal dependencies**. `rustamp-app` (the player) depends on `rustamp-ipc` but **not** on `rustamp-plugin-abi`/`rustamp-plughost`.
**Fix:** extract exactly the types that cross the process boundary into a new permissive crate (call it `rustamp-proto`): `VisFrame` + `VIS_CHANNELS`/`VIS_SAMPLES`, the wire subset of `Command`/`Event`, the wire fields of `PlayerState`, and the playlist/window id types used in messages. `rustamp-core` (GPL) re-exports them (`pub use rustamp_proto::…`) so the player's code and public surface do not change; `rustamp-ipc` (MIT) and `rustamp-plughost` (MIT) then depend only on permissive crates. Extraction (not duplication) is strongly preferred: RFC 0003 requires the shm ring layout to be **byte-identical** on x86 and x86_64 — a single shared definition is the only safe way to keep that invariant. This is a contract change → **RFC required** (`CONTRIBUTING.md`: "Changes to shared contracts (rustamp-core types and traits, the rustamp-ipc protocol, public plugin ABI structs) need a short RFC in docs/rfc/ first").

**(c) Is an independently written ABI/struct definition itself copyrightable subject matter?** For `rustamp-plugin-abi` (written from specs — CLEANROOM.md §4 allows "ABI layouts as facts"):
- **Facts are not copyrightable.** *Feist*, 499 U.S. 340 (1991). The sizes, offsets, constants and call order are facts. [Court-tested]
- **Expression dictated by function is filtered/merged.** *Altai* (abstraction–filtration–comparison): elements required by the interface (a C ABI for a DLL) are not protectable expression. [Court-tested framework]
- **US interface law is unsettled at the top:** *Google v. Oracle* (2021) — SCOTUS assumed declaring-code copyrightability and decided fair use; the Fed. Cir. had held APIs copyrightable. So in the US, an interface's *declaring code* may be protected, but independent reimplementation from facts, and functional layouts, sit at the weak end of protection. [Court-tested as to fair use; unsettled as to copyrightability]
- **EU is clear:** Software Directive 2009/24/EC Art. 1(2)/recital 11 — "ideas and principles which underlie any element of a program, **including those which underlie its interfaces**, are not protected by copyright"; *SAS v. WPL* (CJEU 2012) — functionality, language and data-file formats are not expression. [Court-tested, EU]
- **Interoperability RE is privileged:** 17 U.S.C. §1201(f); *Sega v. Accolade*; *Sony v. Connectix*; Software Directive Art. 5(3)/6. The ABI facts in the specs were obtained by interoperability reverse engineering of the 2.95 binary — the purpose the carve-outs exist for. [Court-tested]
- **Practice:** clean-room API reimplementation is standard and tolerated — Ardour/LMMS "vestige" VST headers, Wine's Windows headers, ReactOS, projectm-eval (an MIT reimplementation of ns-eel2). [Tolerated practice]
- Conclusion: an ABI crate written **from facts in specs** is low-risk, not zero-risk. The residual risk is "substantial similarity of selection/arrangement to the SDK headers" — mitigated by writing from specs, by the merger/functionality arguments, and by the EU rule. **The Winamp plugin SDK's own license terms are a separate, contractual question — deliberately not researched here; the dirty room must answer it (§5, open question 1).**

**(d) Is the MIT host talking to the GPL player over IPC still "separate programs"?** Yes, on the FSF's own framework: separate executables; the control channel is a named pipe ("pipes, sockets and command-line arguments are communication mechanisms normally used between two separate programs"); the data channel is shm with a documented wire layout (the §1.4 caveat applies — which is why the recommendation pairs Option 3 with the §7 permission on the player, so that even the strictest one-program reading is covered for Rustamp's own code). The host is not a work based on the player (it implements a documented protocol; interfaces are not derivative works), and the player is not a work based on the host (MIT grants everything; MIT code may be combined into the GPL player). Distribution of both in one installer: aggregate (GPL-3.0 §5; FSF FAQ #MereAggregation, #GPLCompatInstaller).

**What Option 3 buys:** the process that loads plugin DLLs is not GPL-licensed at all → no copyleft can reach any plugin, of any license, ever — by construction, not by exception. Plugin authors can also **reuse `rustamp-plugin-abi`** (permissive) in their own hosts/plugins (the DeaDBeeF `deadbeef.h` pattern, §2.9). The player keeps full GPL-3.0 copyleft. **Cost:** the `rustamp-proto` refactor, license files/metadata, and the optics discussion (§1.4).

### 3.4 Option 4 — relicense the whole project to LGPL-3.0

- What LGPL-3.0 is: GPL-3.0 plus additional permissions; the license list: "a free software license, but **not a strong copyleft license, because it permits linking with nonfree modules**. It is compatible with GPLv3… not compatible with GPLv2 by itself." [License text/FSF] https://www.gnu.org/licenses/license-list.en.html#LGPLv3 ; LGPL-3.0 §1 ("Exception to Section 3 of the GNU GPL") and the "Combined Work" definitions: https://www.gnu.org/licenses/lgpl-3.0.html
- **Source consequences (§3.7):** copied **GPL-2.0-or-later** code (DeaDBeeF GPL-2.0-or-later plugins; sidplayfp if copied) **cannot** be included: GPL-2.0-or-later offers GPL-3.0, not LGPL-3.0. In-process FFI to sidplayfp (GPL-2.0-or-later) would make the combination GPL-3.0 — defeating the purpose. LGPL-2.1-or-later sources (game-music-emu, projectM) convert fine ("or later") and LGPL libraries may be linked by anything. Permissive sources fine.
- **Does it help the plugin goal?** Only indirectly: if the *host* were LGPL-3.0, its "permits linking with nonfree modules" permission might be argued to cover plugin loading — but the plughost is an executable, not a "Library", and the LGPL's combined-work permission is about linking *with the library*, not about what the library loads. The FSF's linking analysis would still ask whether host+plugin are one program. The separate-process architecture (Option 1/3) is what actually solves it. LGPL-3.0 on an *application* is also off-label: the FSF's "Why you shouldn't use the Lesser GPL for your next library" (linked from the GPL's own "How to apply" section) recommends GPL for applications and LGPL for libraries. https://www.gnu.org/licenses/why-not-lgpl.html
- **Mechanics precedent:** relicensing requires all rightsholders — VideoLAN relicensed libVLC to LGPL in 2011 only after "VideoLAN and VLC developers have completed the process" (all developers agreed). https://images.videolan.org/press/lgpl-libvlc.html
- Verdict: **not recommended.** It forfeits decoder sources (sidplayfp; DeaDBeeF GPL plugin ports), forces a big decoder rearchitecture to keep them, and does not improve the plugin situation beyond Option 3.

### 3.5 Option 5 — relicense the whole project to MIT or Apache-2.0

- **Source consequences (§3.7):** all permissive sources stay (webamp, butterchurn, WDL/EEL2, libopenmpt, WavPack, vis_milk2 BSD-3, ns-eel2 zlib). Copied **GPL-2.0-or-later** and **LGPL-2.1-or-later** code must go (DeaDBeeF GPL/LGPL plugin ports; game-music-emu/projectM if copied). **In-process FFI to sidplayfp (GPL-2.0-or-later) is not permitted from a permissive binary** (dynamic linking = combined work under the GPL — FSF FAQ #GPLStaticVsDynamic): sidplayfp must be dropped, replaced with a permissively licensed SID decoder **[open question: does one exist? §5]**, or moved into a **separate GPL decoder-host process** — the same pattern as the plugin host (§3.7). FFI to LGPL/BSD/MIT libraries (game-music-emu, projectM, libopenmpt, WavPack) is fine: LGPL explicitly permits linking with nonfree modules.
- **MIT vs Apache-2.0:** both are permissive and GPL-compatible; Apache-2.0 is compatible with **GPL-3.0 only** (not GPL-2.0) and adds an explicit patent grant, NOTICE/attribution duties and a trademark clause; the FSF: "For substantial programs it is better to use the Apache 2.0 license **since it blocks patent treachery**." https://www.gnu.org/licenses/license-list.en.html#apache2 and #Expat — For Rust crates, `license = "MIT OR Apache-2.0"` is idiomatic and gives plugin authors both the patent grant and maximum simplicity. (Note: Apache-2.0 code cannot be combined with GPL-2.0-only code — nothing GPL-2.0-only is a Rust dependency in this workspace; sidplayfp is FFI.)
- **Strategic consequence:** the player becomes forkable-into-proprietary; the owner gives up copyleft. That is a values decision, not a legal necessity — and it buys nothing for the plugin goal that Option 3 doesn't buy.
- Verdict: **not recommended** as the primary. (If the owner *wants* a permissive project, this is the honest cost list.)

### 3.6 Option 6 — dual licensing / per-file split

- **Dual licensing** (GPL OR commercial) is a business model, not a plugin fix: Qt ("Qt licensing comes with both commercial and open source options", https://www.qt.io/development/qt-framework/qt-licensing ), MySQL (GPL + commercial). It requires contribution tracking (a CLA or per-contributor copyright) so the vendor can offer the commercial license — today trivially satisfiable (single identity), later a process cost. It does not change what plugins may do.
- **Per-file / per-crate license split** is Option 3's mechanism (and Blender's: Cycles is Apache-2.0 inside a GPL-2.0-or-later project, §2.4). Rustamp already has the right granularity: crates.
- One-liner on AGPL: AGPL-3.0-or-later would extend copyleft to network use; it is irrelevant to local plugins and is itself incompatible with GPL-2.0-only. Not recommended for this goal.

### 3.7 Consequences for the free sources (copied vs FFI matters)

Licenses verified online (URLs in Appendix A; repo-local verification noted):

| Source | License (verified) | How the plan uses it | Opt 1 (GPL, no change) | Opt 2 (GPL + §7 exc) | Opt 3 (permissive plugin crates) | Opt 4 (LGPL-3.0) | Opt 5 (MIT/Apache) |
|---|---|---|---|---|---|---|---|
| webamp (captbaritone/webamp) | **MIT** (LICENSE.txt; GitHub) | copied: sprite tables into `rustamp-skin` (already in `THIRD_PARTY.md`) | keep | keep | keep | keep | keep |
| butterchurn (jberg/butterchurn) | **MIT** (LICENSE) | copied/ported into `rustamp-milkdrop` (M4) | keep | keep | keep | keep | keep |
| WDL / EEL2 (Cockos) | **zlib** (cockos.com/wdl license text) | ported (EEL2 engine in `rustamp-milkdrop`) | keep | keep | keep | keep | keep |
| libopenmpt | **BSD-3-Clause** (lib.openmpt.org/libopenmpt/license/) | FFI in `rustamp-decoders`; `in_openmpt.dll` in CI | keep | keep | keep | keep | keep |
| WavPack | **BSD-3-Clause** (COPYING) | FFI | keep | keep | keep | keep | keep |
| game-music-emu (libgme) | **LGPL-2.1-or-later** (gme.cpp header) | FFI | keep | keep | keep (LGPL may be linked by anything) | keep (FFI fine; copied code converts via "or later") | keep as FFI (copied code would be forfeited) |
| sidplayfp | **GPL-2.0-or-later** (README; msys2) | FFI | keep (player must be GPL-2.0+ compatible — GPL-3.0 is) | keep | keep | **problem**: in-process FFI makes the combination GPL-3.0 (defeats LGPL) | **not permitted in-process** → drop, replace (open question), or separate GPL decoder-host process |
| projectM | **LGPL-2.1-or-later** (COPYING header) | reference only (PLAN: "do not copy without a separate review") | keep | keep | keep | keep | keep as FFI/reference |
| DeaDBeeF — zlib core | **zlib** (README §3; src/main.c header) | copied (decoder patterns) | keep | keep | keep | keep | keep |
| DeaDBeeF — GPL-2.0-or-later plugins (e.g. cdda, ffmpeg wrappers) | **GPL-2.0-or-later** (plugin headers) | copied (only if ported) | keep (via "or later" → GPL-3.0) | keep | keep | **forfeit** (GPL-2.0+ does not offer LGPL-3.0) | **forfeit** if copied |
| DeaDBeeF — zlib plugins (adplug, dumb, gme) | **zlib** (plugin headers) | copied | keep | keep | keep | keep | keep |
| MilkDrop 2.25c `vis_milk2/` | **Nullsoft BSD-3-Clause** (verified in `milkdrop_225c_src.zip`: "Copyright 2005-2013 Nullsoft, Inc.… Neither the name of Nullsoft nor the names of its contributors may be used…") | copied (M4 native MilkDrop; SDK headers in the zip stay dirty) | keep | keep | keep | keep | keep |
| ns-eel2 (in the MilkDrop zip) | **zlib-style headers** in `ns-eel.h`, `ns-eel-int.h`, `nseel-compiler.c`, `nseel-eval.c`, `nseel-cfunc.c`, `ns-eel-addfuncs.h` ("Copyright (C) 1999-2003 Nullsoft, Inc." / "Copyright (C) 2004-2008 Cockos Incorporated"); **the `asm-nseel-*.c` files show no license header** → per-file check before copying (CLEANROOM.md §7) | copied (M4 expression engine) | keep (per-file header check) | keep | keep | keep | keep |

Notes:
- "Copied" = code enters a Rustamp crate and must be license-compatible with that crate. "FFI" = a shared library keeps its own license; the app's license interacts via the LGPL/GPL linking rules (GPL library → the combined work must be GPL-compatible; LGPL library → the app may be anything).
- LGPL-2.1-or-later → GPL-3.0 conversion is permitted (LGPL-2.1 §3: "If a newer version than version 2 of the ordinary GNU General Public License has appeared, then you can specify that version instead if you wish."), which is why game-music-emu works in a GPL-3.0 player. https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
- A forfeited source can sometimes be kept **out of process**: the plugin-host pattern (a separate GPL process speaking a documented protocol) works for decoders too — at real architectural cost. That is the fallback if the owner ever chooses Option 5 and still wants SID support.
- DeaDBeeF plugin licenses are **mixed per plugin** (zlib for some, GPL-2.0-or-later for others) — check each plugin at copy time (§5, open question 3).

### 3.8 What each option buys for the plugin goal

| Goal | Opt 1 | Opt 2 | Opt 3 (+2) | Opt 4 | Opt 5 |
|---|---|---|---|---|---|
| Run proprietary plugins (user-installed) | yes | yes | yes | yes | yes |
| Run GPL-2.0-only plugins | yes (separate process) | yes | yes | yes | yes |
| Run GPL-3.0 / permissive plugins | yes | yes | yes | yes | yes |
| Plugin authors get an explicit grant / reusable ABI | no | partial (§7, removable) | **yes** (MIT host + MIT ABI crate + §7 on player) | partial | yes |
| Bundle plugins with the host/player | only GPL-3.0-compatible plugins | as to our code, yes; GPL-2.0-only plugins still conflict | **yes for the MIT host** (host imposes nothing); player+plugin = aggregate | same as 3 | same as 3 |
| Player keeps full copyleft | yes | yes | **yes** | no (weak copyleft) | no |
| Keeps all planned decoder sources | yes | yes | **yes** | no (sidplayfp, DeaDBeeF GPL ports) | no (same, plus FFI constraints) |

---

## 4. Recommendation

### 4.1 Primary recommendation: Option 3 + Option 2 combined

**Permissively license the plugin-facing surface; keep the player GPL-3.0-or-later; and add a GPL-3.0 §7 plugin-combination additional permission to the GPL crates.** Concretely:

1. `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` and a new `rustamp-proto` crate → **`license = "MIT OR Apache-2.0"`** (per-crate override of `license.workspace = true`), each with its own LICENSE file and a header notice. The player and all other crates stay **GPL-3.0-or-later**.
2. Decouple: move the wire types (`VisFrame` + constants, the `Command`/`Event` wire subset, `PlayerState` wire fields, `EntryId`/`MediaLocation`/`SortKey`/`WindowId` as used by the protocol) from `rustamp-core` into `rustamp-proto`; `rustamp-core` re-exports them; `rustamp-ipc`/`rustamp-plughost` depend only on permissive crates. (Contract change → RFC.)
3. Add the §7 additional permission to the GPL crates that contain only Rustamp's own code (at minimum `rustamp-core` and the crates that make up `rustamp.exe`; **not** crates carrying third-party GPL code such as `rustamp-decoders`), tied to the documented plugin ABI and IPC protocol, modeled on the FSF's #LinkingOverControlledInterface text.

**Reasoning.** The plugin boundary is decided by the license of the process that loads the DLLs (§1.6). Making that process permissive removes the GPL from the plugin edge *structurally* — no combined-work theory can even start, for plugins of any license, including GPL-2.0-only (§1.5). The §7 permission on the player covers the player↔host edge (the shm hook, §1.4) for Rustamp's own code, and gives plugin authors and downstream users an explicit, citable grant. The two instruments cover each other's weak points: the license is durable (not removable, unlike a §7 permission), and the permission covers the edge the license doesn't reach (the GPL player itself). The player — the actual creative work — keeps full copyleft. Nothing in the plan's source list is forfeited (§3.7). And the whole change is nearly free **right now** (single rightsholder, unpublished repo), which is the best moment in the project's life to do it.

**Suggested exception text** (adapt the FSF's model; attach per crate):

> As an additional permission under section 7 of the GNU General Public License, version 3 (or, at your option, any later version), the copyright holders of Rustamp give you permission to combine the covered work with independent modules that communicate with the covered work solely through (a) the documented Rustamp classic-plugin ABI (the Winamp 2.x `in_`/`out_`/`vis_`/`dsp_`/`gen_` plugin interface, as defined by the `rustamp-plugin-abi` crate) and (b) the documented Rustamp IPC protocol (the `rustamp-ipc` crate, protocol version 1), regardless of the license terms of those independent modules, and to copy and distribute the resulting combination under the terms of the GNU GPL for the covered work and the licenses of the other code concerned, provided that you do not modify those documented interfaces. This exception does not apply to material in the covered work that is licensed under the GPL by third parties; as to that material, the GPL applies without this exception.

### 4.2 Fallback: Option 2 alone (uniform GPL-3.0 + §7 plugin exception)

If the owner prefers to keep the project uniformly GPL-3.0 (no relicensing at all): keep every crate GPL-3.0-or-later and add the §7 plugin-combination additional permission (§4.1 text) to the plugin-facing crates (`rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` — all our code today, so we may authorize the exception for them, per FSF FAQ #GPLIncompatibleLibs) **and** to `rustamp-core`/the player crates, plus a published plugin-licensing clarification modeled on Ardour's Plugin Clarification (§2.2). No decoupling refactor is needed (the host may keep linking GPL crates, since it stays GPL). Accept the caveats: the permission is removable by downstream conveyors (GPL-3.0 §7), modified versions need not carry it forward, and the FSF's #GPLPlugins analysis still describes host+plugin as a combined program with the exception as the only bridge. If even the license-notice change is unwanted, the do-nothing baseline is Option 1 (§3.1) plus the published clarification — with the risks stated there.

### 4.3 Risk assessment

| Risk | Level | Mitigation |
|---|---|---|
| FSF/court treats player+host as one program (shm hook: "Using shared memory to communicate with complex data structures is pretty much equivalent to dynamic linking") | Low–Medium [FSF interpretation; untested] | §7 permission on the player tied to the documented IPC protocol; bitness-forced split (Microsoft: a 64-bit process cannot load a 32-bit DLL); documented, versioned wire protocol; crash isolation; GIMP precedent (§2.1) |
| "Designed to evade the GPL" perception | Medium (reputational) | The split is technically forced and predates the licensing question; the player stays GPL; the decision is documented openly in an RFC; GIMP/Ardour/OBS precedents; the FSF's own controlled-interface exception blesses the goal (§1.4) |
| §7 permission stripped by a downstream fork/distro | Low (GPL-3.0 §7 allows removal) | The permissive license on the host is **not** removable — it is the primary instrument; the permission is the companion |
| Third-party GPL-2.0+ code inside the player (sidplayfp via FFI) cannot be covered by our exception | Low (only under the strictest one-program theory) | The host is permissive and separate; optional future hardening: move GPL-2.0+ decoders into a separate decoder-host process (the plughost pattern), or replace sidplayfp with a permissive SID decoder (open question) |
| Winamp plugin SDK's own license terms restrict a third-party host or ABI implementation | Unknown | **Open question 1 — dirty room must answer; deliberately not researched here** |
| Trademark ("Winamp") | Out of scope | The project already avoids the name; not a code-license question |
| Patent claims covering the plugin ABI | None known | Apache-2.0's explicit patent grant for the plugin-facing crates; FSF: Apache-2.0 "blocks patent treachery" |
| Plugin authors still refuse | Low | MIT host + MIT ABI crate + §7 permission + published clarification (Ardour pattern) is the strongest assurance available short of relicensing everything |
| Relicensing window missed (repo published / external PRs arrive first) | Medium if delayed | Decide now; add inbound=outbound licensing to `CONTRIBUTING.md` + PR template before the first external PR |

### 4.4 Migration mechanics (what changes, file by file)

1. **`docs/rfc/0004-plugin-licensing.md`** (new): the RFC for the contract change (moving wire types out of `rustamp-core` into `rustamp-proto`) and the license split. Required by `CONTRIBUTING.md` before any contract change.
2. **`crates/rustamp-proto/`** (new crate, `license = "MIT OR Apache-2.0"`): the wire types moved from `rustamp-core` (`vis` module, `Command`/`Event` wire subset, `PlayerState` wire fields, `EntryId`/`MediaLocation`/`SortKey`/`WindowId` as used by the protocol). Keep the RFC 0003 layout asserts (byte-identical x86/x86_64). `rustamp-core` re-exports (`pub use rustamp_proto::…`) and depends on it (MIT → GPL direction: fine).
3. **`crates/rustamp-plugin-abi/Cargo.toml`, `crates/rustamp-ipc/Cargo.toml`, `crates/rustamp-plughost/Cargo.toml`**: override `license.workspace = true` with `license = "MIT OR Apache-2.0"`; drop `rustamp-core` from `rustamp-ipc`/`rustamp-plughost` dependencies (use `rustamp-proto`); add LICENSE files (or a `LICENSES/` directory referenced per crate) and a one-line notice in each crate root.
4. **§7 additional permission**: add `PLUGIN-EXCEPTION.md` (text in §4.1) to the GPL crates containing only our code (at minimum `rustamp-core`, and the crates that make up `rustamp.exe`), referenced from each crate root; record in each crate's README that the exception applies to Rustamp's own contributions only.
5. **`THIRD_PARTY.md`**: add a "Licensing of Rustamp itself" section: the split (player GPL-3.0-or-later; `rustamp-proto`/`rustamp-plugin-abi`/`rustamp-ipc`/`rustamp-plughost` MIT OR Apache-2.0), the §7 exception text and where it applies, and the statement that the permissive crates contain no third-party code.
6. **`docs/PLAN.md` §1/§2**: note the license split in the source table and add a short "Licensing" paragraph to the architecture section (player GPL; plugin-facing crates permissive; host binary permissive; why; pointer to the RFC and this note).
7. **`CONTRIBUTING.md` + `.github/pull_request_template.md`**: add an inbound=outbound licensing statement ("by contributing to crate X you license your contribution under X's license") — **before the first external PR**; without it, any future relicensing must chase contributors.
8. **Plugin-author-facing doc** (e.g. `docs/plugin-licensing.md` or a README section), modeled on Ardour's Plugin Clarification: plugins of any license may be loaded; the host is MIT OR Apache-2.0; the ABI crate is MIT OR Apache-2.0 and may be reused; the player is GPL-3.0-or-later with a §7 permission; plugins are separate programs; Rustamp does not distribute plugins.
9. **Packaging (`xtask package`)**: ship `rustamp.exe` (GPL-3.0-or-later) + `rustamp-plughost32.exe` (MIT OR Apache-2.0) in one NSIS installer — an aggregate (GPL-3.0 §5; FSF FAQ #MereAggregation, #GPLCompatInstaller). Plugins are never bundled; if a specific plugin is ever bundled, clear its license individually.
10. **CI**: the plugin compatibility matrix is unchanged (running plugins is unrestricted; and the host is now permissive anyway). Optionally add a license-metadata check (e.g. `cargo metadata`/`cargo-deny`) asserting that `rustamp-proto`/`rustamp-plugin-abi`/`rustamp-ipc`/`rustamp-plughost` have no GPL-licensed dependencies.
11. **i686 build**: unchanged — the permissive crates build for both targets; `cargo xtask ring-cross` keeps validating the byte-identical ring layout.

### 4.5 What the plugin host's distribution looks like

`rustamp-plughost32.exe` (i686-pc-windows-msvc): **MIT OR Apache-2.0**, source in the same repo (`crates/rustamp-plughost` + `rustamp-proto` + `rustamp-ipc` + `rustamp-plugin-abi`), shipped inside the GPL installer next to `rustamp.exe` (aggregate) and downloadable standalone. It loads any classic `in_`/`out_`/`vis_`/`dsp_`/`gen_` DLL regardless of that DLL's license; the plugin's own license governs the plugin; no GPL obligation flows from the host to any plugin; crash isolation (restart on plugin crash) is unchanged. Plugin authors may reuse `rustamp-plugin-abi` in their own tools under MIT OR Apache-2.0.

---

## 5. Open questions

1. **The Winamp plugin SDK's own license terms.** Whether Nullsoft's SDK license restricts implementing the plugin ABI in a third-party host, or loading its plugins, is a contractual/trademark/patent question beyond copyright's derivative-work analysis. **Deliberately not researched here — the dirty room must answer it (do not read the SDK headers in the clean room).** This matters for whether a third-party host may load SDK-built plugins at all.
2. **Exact license terms of the classic plugin binaries Rustamp wants to run/support:** the MilkDrop 2 binary as shipped in Winamp 5.66; the Geiss 4.29 / Geiss 2 binaries; Nullsoft's bundled `in_*`/`out_*` plugins (closed-source freeware; individual terms not published). Also: a census of whether any specific classic plugin is **GPL-2.0-only** (AVS = BSD-style since 2005, `in_openmpt` = BSD-3 via the libopenmpt project, and MilkDrop 2.25c `vis_milk2` source = Nullsoft BSD-3 are confirmed; the rest is unverified).
3. **DeaDBeeF per-plugin grants:** confirmed zlib (adplug, dumb, gme plugins) and GPL-2.0-or-later (cdda, ffmpeg plugins); the licenses are mixed per plugin — check each plugin at copy time.
4. **ns-eel2:** zlib-style headers verified in `ns-eel.h`, `ns-eel-int.h`, `nseel-compiler.c`, `nseel-eval.c`, `nseel-cfunc.c`, `ns-eel-addfuncs.h`; the `asm-nseel-*.c` files show no license header in their first lines — per-file verification before copying (CLEANROOM.md §7). This resolves PLAN §1's "check ns-eel2 separately" as far as the zip's own headers go.
5. **Does a permissively licensed SID decoder exist** (fallback for sidplayfp if the owner ever chooses Option 5)?
6. **Does a §7 additional permission survive an "or later" election** to a future GPL version? The FSF FAQ is silent. **[For a lawyer.]** Mitigation: word the exception to apply under any chosen version (§4.1).
7. **Patent landscape for the Winamp plugin ABI:** none known; the Apache-2.0 patent grant covers the plugin-facing crates.
8. **Trademark status of "Winamp"** in the relevant jurisdictions — out of scope for this report; the project already avoids the name.
9. **CLA vs inbound=outbound:** whether the owner wants a formal CLA or the simpler "contributions are licensed under the crate's license" statement — decide before the first external PR (§4.4, item 7).

---

## 6. Not legal advice

This report is licensing research, not legal advice. The FSF's own FAQ says the same about its publications: "they are not legal advice. The FSF cannot give legal advice." https://www.gnu.org/licenses/gpl-faq.en.html The plugin/process boundary has **no court-tested answer** in any jurisdiction; the positions quoted from the FSF are one party's interpretation, and the precedents in §2 are tolerated practice, not rulings. Before acting on the recommendation, the project owner should have a lawyer (one familiar with software copyright and GPL enforcement — the FSF's licensing team at licensing@gnu.org can refer, and several firms specialize in GPL work) review: (a) the §7 exception text and its interaction with "or later"; (b) the Winamp SDK license question (open question 1); (c) the trademark question; and (d) the choice between the primary recommendation and the fallback.

---

## Appendix A: source register

### A.1 License texts
- GPL-3.0: https://www.gnu.org/licenses/gpl-3.0.html (repo `LICENSE` is identical)
- LGPL-3.0: https://www.gnu.org/licenses/lgpl-3.0.html
- LGPL-2.1: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html (§3 conversion clause quoted in §3.7)
- Apache-2.0 / MIT (Expat) / zlib entries: https://www.gnu.org/licenses/license-list.en.html#apache2 , #Expat , #Zlib
- EU Software Directive 2009/24/EC (Art. 1(2), recital 11, Art. 5(3), 6): https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32009L0024

### A.2 FSF guidance
- GNU GPL FAQ: https://www.gnu.org/licenses/gpl-faq.en.html — anchors used: #GPLPlugins, #GPLAndPlugins, #NFUseGPLPlugins, #LinkingOverControlledInterface, #MereAggregation, #AggregateContainers, #WhatIsCompatible, #GPLCompatInstaller, #GPLStaticVsDynamic, #IfLibraryIsGPL, #SystemLibraryException, #GPLIncompatibleLibs, #GPLInProprietarySystem, #WillYouMakeAnException, #v2v3Compatibility, #AllCompatibility, #GPLModuleLicense, #GPLOutput, #WhatCaseIsOutputGPL, #InterpreterIncompat, #WhoHasThePower, #FontException, #LibGCCException, #GPLIncompatibleAlone, #VersionThreeOrLater; plus the question "Where's the line between two separate programs, and one program with two parts?" (section *Combining work with code released under the GNU licenses*)
- FSF license list: https://www.gnu.org/licenses/license-list.en.html — anchors: #GPLv2, #GNUGPLv3, #LGPLv2.1, #LGPLv3, #apache2, #Expat, #Zlib, #GPLCompatibleLicenses, #GPLIncompatibleLicenses
- Why not LGPL: https://www.gnu.org/licenses/why-not-lgpl.html
- Classpath exception: https://www.gnu.org/software/classpath/license.html
- GCC Runtime Library Exception 3.1: https://www.gnu.org/licenses/gcc-exception-3.1.en.html
- MySQL FLOSS exception: https://www.mysql.com/about/legal/licensing/foss-exception/ (text via https://lists.debian.org/debian-legal/2004/05/msg00900.html)

### A.3 Case law and statutes
- *Jacobsen v. Katzer*, 535 F.3d 1373 (Fed. Cir. 2008): https://jolt.law.harvard.edu/digest/jacobsen-v-katzer ; https://www.law.berkeley.edu/archive/files/Jacobsen_v_Katzer_-_Westlaw.pdf
- *Feist Publications v. Rural Telephone*, 499 U.S. 340 (1991): https://supreme.justia.com/cases/federal/us/499/340/
- *Computer Associates v. Altai*, 982 F.2d 693 (2d Cir. 1992): https://law.justia.com/cases/federal/appellate-courts/F2/982/693/137252/
- *Google LLC v. Oracle America*, 593 U.S. 1 (2021): https://www.supremecourt.gov/opinions/20pdf/18-956_d18f.pdf ; https://www.law.cornell.edu/supremecourt/text/18-956
- *SAS Institute Inc v World Programming Ltd*, CJEU C-406/10, ECLI:EU:C:2012:259 (2 May 2012): https://en.wikipedia.org/wiki/SAS_Institute_Inc_v_World_Programming_Ltd ; http://curia.europa.eu/juris/liste.jsf?num=C-406/10
- *Sega Enterprises v. Accolade*, 977 F.2d 1510 (9th Cir. 1992): https://www.copyright.gov/fair-use/summaries/segaenters-accolade-9thcir1992.pdf
- *Sony Computer Entertainment v. Connectix*, 203 F.3d 596 (9th Cir. 2000): https://law.justia.com/cases/federal/appellate-courts/F3/203/596/474793/
- 17 U.S.C. §1201(f): https://www.law.cornell.edu/uscode/text/17/1201
- Microsoft, "Process Interoperability" (64-bit process cannot load a 32-bit DLL): https://learn.microsoft.com/en-us/windows/win32/winprog64/process-interoperability

### A.4 Precedents
- GIMP: license https://gitlab.gnome.org/GNOME/gimp/-/raw/master/COPYING , header https://gitlab.gnome.org/GNOME/gimp/-/raw/master/app/core/gimp.c , Fedora https://packages.fedoraproject.org/pkgs/gimp3/gimp3/ ; plug-ins: https://docs.gimp.org/2.10/en/gimp-scripting.html , https://developer.gimp.org/resource/writing-a-plug-in/tutorial-c-basic/
- Ardour: COPYING + Plugin Clarification https://raw.githubusercontent.com/Ardour/ardour/master/COPYING
- Audacity: LICENSE.txt https://raw.githubusercontent.com/audacity/audacity/master/LICENSE.txt , about/license https://www.audacityteam.org/about/license/ , VST manual https://manual.audacityteam.org/man/effect_menu_vst.html ; Steinberg VST3 MIT (Oct 2025) https://www.steinberg.net/press/2025/vst-3-8/
- Blender: https://www.blender.org/about/license/
- WordPress: https://wordpress.org/about/license/
- LMMS: header https://raw.githubusercontent.com/LMMS/lmms/master/src/core/Engine.cpp ; VeSTige https://docs.lmms.io/user-manual/instruments/vestige
- VLC: README https://github.com/videolan/vlc/blob/master/README.md ; press releases https://images.videolan.org/press/lgpl.html , https://images.videolan.org/press/lgpl-libvlc.html
- Wine: header https://raw.githubusercontent.com/wine-mirror/wine/master/dlls/kernel32/process.c ; COPYING.LIB https://gitlab.winehq.org/wine/wine/-/raw/master/COPYING.LIB
- OBS Studio: https://github.com/obsproject/obs-studio ; forum position https://obsproject.com/forum/threads/new-plugin.142141/
- DeaDBeeF: README §3 https://raw.githubusercontent.com/DeaDBeeF-Player/deadbeef/master/README ; API header (zlib) https://raw.githubusercontent.com/DeaDBeeF-Player/deadbeef/master/include/deadbeef/deadbeef.h ; plugin headers (cdda GPL-2.0-or-later; adplug/dumb/gme zlib) https://github.com/DeaDBeeF-Player/deadbeef/tree/master/plugins ; AUR https://aur.archlinux.org/packages/deadbeef-git
- Mozilla MPL-2.0 FAQ: https://www.mozilla.org/en-US/MPL/2.0/FAQ/
- VirtualBox: https://www.virtualbox.org/wiki/Licensing_FAQ
- Qt dual licensing: https://www.qt.io/development/qt-framework/qt-licensing
- Winamp 2024 source release (non-free license): https://hackaday.com/2024/09/24/winamp-releases-source-code-but-is-it-really-open/
- AVS (BSD-style since May 2005): https://sr.ht/~grandchild/AVS/ , https://getmusicbee.com/addons/visualizer/367/advanced-visualization-studio/

### A.5 Free sources (licenses verified)
- webamp MIT: https://raw.githubusercontent.com/captbaritone/webamp/master/LICENSE.txt ; https://github.com/captbaritone/webamp
- butterchurn MIT: https://raw.githubusercontent.com/jberg/butterchurn/master/LICENSE
- WDL zlib: https://www.cockos.com/wdl/
- libopenmpt BSD-3-Clause: https://lib.openmpt.org/libopenmpt/license/
- WavPack BSD-3-Clause: https://raw.githubusercontent.com/dbry/WavPack/master/COPYING
- game-music-emu LGPL-2.1-or-later: https://raw.githubusercontent.com/libgme/game-music-emu/master/gme/gme.cpp (header)
- sidplayfp GPL-2.0-or-later: https://github.com/libsidplayfp/sidplayfp (README); https://packages.msys2.org/package/mingw-w64-ucrt-x86_64-libsidplayfp
- projectM LGPL-2.1-or-later: https://raw.githubusercontent.com/projectM-visualizer/projectm/master/COPYING
- MilkDrop 2.25c `vis_milk2/` Nullsoft BSD-3-Clause: verified in `D:\code\rustamp-refbin\geiss-site\milkdrop_225c_src.zip` (headers of `vis_milk2/config.cpp`, `config2.cpp`); source https://www.geisswerks.com/milkdrop/milkdrop_225c_src.zip
- ns-eel2 zlib-style headers: verified in the same zip (`ns-eel2/ns-eel.h`, `ns-eel-int.h`, `nseel-compiler.c`, `nseel-eval.c`, `nseel-cfunc.c`, `ns-eel-addfuncs.h`); `asm-nseel-*.c` unmarked — per-file check

### A.6 Repo facts (local)
- `docs/PLAN.md` §1–§2, §5; `CLEANROOM.md` §1, §4, §7; `THIRD_PARTY.md`; `docs/HANDOFF.md`; `LICENSE`; `Cargo.toml` (workspace `license = "GPL-3.0-or-later"`); `crates/*/Cargo.toml` (deps of `rustamp-plughost`, `rustamp-ipc`, `rustamp-plugin-abi`, `rustamp-app`); `rustamp-ipc/src/message.rs`, `src/transport.rs`, `src/shm/header.rs`, `src/shm/ring.rs`, `examples/ring_peer.rs` (uses of `rustamp_core`); `CONTRIBUTING.md` (RFC rule; no CLA); git: 18 commits, single identity `Rustamp <rustamp@project>`, no remote.
