---
Spec ID: S-21
Title: What a Winamp 5.x plug-in expects of its host (start-up services)
Sources consulted (category only): dirty room: static analysis (byte parsing and a 32-bit x86 disassembler, nothing executed) of the fourteen plug-in DLLs extracted from the Winamp 5.66 installer; dirty room: the survey output of the plug-in survey run in the Hyper-V guest (two runs, 2026-10-25, identical apart from timings); clean repo: audited S09 and S10 and the host's declared getter signature, read-only context
Author agent: dirty-room spec agent, lane S21 (RFC 0021)
Auditor: spec auditor subagent (lane S21)
Audit date: 2026-10-25
Dirty-room authorisation: RFC 0021 - the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: plug-ins of the Winamp 5.66 installer (the 2.95 answers of S10 are unchanged and stay the default)
Status: audited (with changes; see the audit note S21-audit.md in the lane folder)
---

# S21 - what a Winamp 5.x plug-in expects of its host

## 0. How to read this spec

No plug-in was run on the host; the VM survey is the only runtime evidence.
No statement here was measured against a host that gives the 5.x answers - such
a host does not exist yet. The statements about what a plug-in does with an
answer come from reading the plug-in's own binary; they are deterministic but
were not run.

Evidence tags. The last letter in brackets is the auditor's mark.

| Tag | Meaning |
|---|---|
| **measured** | Seen at run time in the VM survey (call trace, fault address, message numbers). |
| **binary** | Read from the DLL's bytes (static data, import table, the path the code takes). |
| **reading** | A hypothesis built on the above; an implementer must not rely on it without a measurement. |
| [A] | The auditor re-derived the statement from the binaries. |
| [N] | The auditor could not (or did not) re-derive it; it is the draft's statement, kept as written. |
| [C] | The auditor corrected the draft's statement; the text is the corrected one. |

Addresses are offsets (RVA) from the DLL's load address. Fault addresses of
the survey were taken at the preferred load address of each DLL, so RVA =
reported address minus the preferred base.

### Names used in this spec

The DLLs identify the objects below only by 16-byte identifiers. The role
names are **neutral names given by this spec**; they are not names from any
vendor header.

| Name here | Meaning |
|---|---|
| service manager | The object a plug-in is given (or asks for) that hands out services. |
| service factory | What the service manager returns for a service identifier; it hands out the service object. |
| service object | The object that implements one service. |
| language service | The service that loads text from a plug-in's own string table and starts "language support". |
| configuration service | A service that returns grouped settings (integers, booleans, strings by name). |
| application service | A service the dialog code uses (mouse-wheel routing, a boolean query). |
| memory service | A service that allocates memory the host later releases. |
| generic call | The single call every one of these objects offers (section 3). |
| main window | The window handle the host puts in the module struct, or that a header getter receives. |
| IPC n | The window message 0x400 sent to the main window with the number n in the lparam (wparam as shown). |

The export names `winampGetInModule2`, `winampGetOutModule`,
`winampGetGeneralPurposePlugin`, `winampDSPGetHeader2` and `winampVisGetHeader`,
the IPC numbers and the module-struct offsets of S09 are the public contract the
plug-ins depend on and are used as they are.

## 1. Answers in one page

| # | Question | Answer | Evidence |
|---|---|---|---|
| 1 | What does each of the seven plug-ins read through null (or 1), and where does the value come from? | Three sources, nothing else: **(a)** a *service manager pointer* the plug-in expects to be valid and whose first table entry it calls: for the three input plug-ins it is a **member of the module struct at offset 0x98** that the host must fill, for the others it is the **answer to IPC 3025**; **(b)** the **window handle that the host must pass to the header getter** of visualization and DSP plug-ins (the host passes none, so the plug-ins ask a dead window and get 0); **(c)** for `dsp_sps`, **the answer to IPC 336 used as a string pointer**. Table in section 7. | measured (fault sites) + binary (what the code does) [A] |
| 2 | What does 3025 return on a 5.x host and which services are looked up? | A pointer to the **service manager object** (section 3). At start-up the plug-ins look up the language service (all eleven that use services; unchecked for the seven that fail), and, null-checked, the configuration, application and memory services. Section 5. | binary [A] |
| 3 | Meaning of 334, 336, 3015 (and 1334/1335) | 334 = pointer to an ANSI string used as the **file name of the private-profile (INI) calls**; 336 = pointer to an ANSI string used as a **directory prefix** (`dsp_sps` appends a fixed sub-directory name and creates it); 1334 = wide string used as the file name of the wide private-profile calls; 1335 = wide string combined with the literal `genres.txt` (a directory); 3015 = **two function pointers** selected by wparam (0: integer random generator, 1: floating-point random generator), both called with no arguments. Section 4. Which real-world path each should be is a **reading**. | binary [A], meaning of the paths reading |
| 4 | Module-struct fields a 5.x host fills that a 2.x host does not | One: the **service manager pointer at offset 0x98 of the input module**. The three 5.x input plug-ins carry the version value 0xFF000101. Output, DSP and visualization modules have no new member; but the host must (i) read the description pointer **after** init/getter because the plug-in rewrites it, as **wide** text where it uses wide text, and (ii) pass the main window to the header getters. Section 6. | binary [A] |
| 5 | Plug-in with no service manager: does it start? | `in_wave`, `in_flac`, `in_vorbis` (and `out_disk`, `dsp_sps`, `vis_avs`, `vis_milk2`) **do not start**: they call through the pointer unchecked. Only `gen_hotkeys`, `gen_jumpex` and `out_wave` test the 3025 answer for 0 and 1 and start without one, in a degraded mode. Nothing is printed or shown. What a real 5.x host does cannot be read from the plug-ins (open, section 11). | binary [A for `in_wave`, `in_vorbis`, `out_disk`, `out_wave`, `gen_hotkeys`, `vis_avs`, `vis_milk2`, `dsp_sps`; N for `in_flac` beyond the fault site and `gen_jumpex`] |

## 2. How the host tells which kind of host it is talking to

The 2.95 reference answers 3015 and 3025 with 1 (S10). That stays true for
**anything that is not one of the plug-ins below**.

| Fact | Evidence |
|---|---|
| IPC 3015 and 3025 and every service call are issued **by the plug-in's own code**; none of the 2.x-era interfaces in S09 sends them. A 2.x plug-in cannot observe the answer. | reading [N] |
| The header version does **not** reliably mark a 5.x-aware plug-in: `vis_avs` carries visualization header version 0x101 and `vis_milk2` 0x102 and both ask 3025 first thing; `gen_hotkeys` and `gen_jumpex` carry general-purpose version 0x11, `out_disk` carries output version 0x11. | binary [A]; `vis_nsfs` (0x101) and `out_wave` (0x11) [N] |
| The version value **does** mark the input modules that need the 0x98 member: all three carry 0xFF000101. | binary [A] |
| Visualization and DSP getters are called **before** the host can look at any version, and they use their argument at once. | binary [A] |

Consequence (reading, for the orchestrator to decide): a host that answers
3025, 3015, 336, 334, 1334 and 1335 with the 5.x shapes **to messages sent by
plug-in code inside the plug-in host process** cannot disturb a 2.x plug-in and
needs no version test for outputs, general-purpose, DSP and visualization
plug-ins. For input plug-ins the version value decides whether member 0x98 is
filled: for a plain 0x100 module the struct has 38 members and 0x98 lies past
its end, so it must stay untouched there. The external-probe answers of S10
(sender is not a plug-in) stay as they are.

## 3. The service manager object (the one thing all seven need)

A plug-in that has a service manager calls it, and every service factory and
service object it gets back, through **one generic call**. [A]

| Item | Fact | Evidence |
|---|---|---|
| Object layout | The object's first 4 bytes hold a pointer to a table; **entry 0 of the table** is the generic call. No other table entry is used by any caller seen. | binary [A] for all callers examined, [N] for the claim "no other entry anywhere" |
| Calling convention | 32-bit, object pointer in ECX (the member-call convention of the 32-bit C++ compiler), four 32-bit arguments on the stack, **the callee removes them** (16 bytes); callers do not adjust the stack afterwards. The plug-ins' own implementations of the same call end by removing 16 bytes. | binary [A] |
| Argument 1 | Message number (an int). | binary [A] |
| Argument 2 | Pointer to a 32-bit cell that receives the **result value** (pointer or int). | binary [A] |
| Argument 3 | Pointer to an array of **pointers to the argument values**: each argument is passed by address; a 16-byte identifier is passed as a pointer to a 16-byte copy. | binary [A] |
| Argument 4 | Number of arguments in that array. | binary [A] |
| Return value | **Nonzero = understood**, 0 = not understood. Every caller turns "not understood" into a zero result; the result cell is ignored then. A host may therefore answer any message it does not know with 0. | binary [A] for the callers; the "own implementations return 1 / write the cell only when handled" part [N] |
| Thread | Start-up calls come from the plug-in's UI thread; later calls (configuration, memory) also from worker threads. | reading [N] |

Messages used **on the service manager itself**:

| Msg | Meaning | Arguments | Result | Used by | Start-up |
|---|---|---|---|---|---|
| 10 | **Register a service factory** the plug-in provides | 1: pointer to the plug-in's factory object | ignored by every caller seen | `in_wave`, `in_flac`, `in_vorbis` | Unconditional; **crashes on a null manager**. Either 1 or 0 may be returned. [A] |
| 20 | **Deregister** that factory | 1: same pointer | ignored | the same three, in `quit` | `quit` only [A for `in_wave` and `in_vorbis`] |
| 50 | **Look up a service by identifier** | 1: 16-byte identifier | pointer to a **service factory**, or 0 | all plug-ins that use services | Yes; 0 is tolerated for every service except the language service in the seven that fault [A] |

Messages used **on the service factory** returned by 50:

| Msg | Meaning | Arguments | Result |
|---|---|---|---|
| 300 | Get the service object | 1: int, always 1 | pointer to the service object [A] |
| 310 | Release the service object | 1: pointer returned by 300 | ignored (in `quit`) [N] |

The factory and the service object have the same shape (table entry 0 = generic
call). The plug-ins never call anything else on a factory. The factories the
**plug-ins register** (message 10) are plug-in-provided; **nothing at start-up
requires the host to call into them**. [A for `in_wave`]

**The least a host must provide so that a plug-in starts:** a service manager
whose generic call returns 1 for message 10, and for message 50 returns a
factory for the **language service** when asked for its identifier (section 5)
and 0 for every other identifier; that factory answers 300 with a language
service object (section 5.1). (reading, from the null checks; not measured.)

## 4. IPC requests the plug-ins send at start-up

All are window message 0x400 sent to the main window. "Id" is the lparam.

| Id | wparam | What the plug-in does with the answer | Shape it needs | Who | Evidence |
|---|---|---|---|---|---|
| **334** | 0 | Used as the **file name argument of the private-profile calls** (the plug-in's own settings section). | Pointer to a NUL-terminated **ANSI string** that stays valid for the whole session. A path of a file that does not exist yet is fine for these calls (reading). | `in_wave`, `in_vorbis`, `out_disk`, `out_wave`, `dsp_sps` (after 336), `vis_avs` (module init, buffer of 260 characters) | measured (sent by `out_disk`; answered 1; the fault came later at 3025); binary [A for `in_wave`, `out_disk`, `dsp_sps`, `out_wave` send site, `in_vorbis`; N for `vis_avs` use] |
| **336** | 0 | `dsp_sps` is the first thing in its init: formats the answer with a `%s` and the literal `\dsp_sps` and **creates that directory**. A value of 1 makes the system formatting routine read address 1 and fault. | Pointer to a NUL-terminated **ANSI directory path**; the directory must be writable. That it should be the plug-ins folder is a reading. | `dsp_sps` | measured (fault inside the system formatting routine, reading 0x1, right after the 336 message) + binary [A] |
| **1334** | 0 | File name as in 334, **wide**. | Pointer to a NUL-terminated wide string. | `in_flac` (wide private-profile read), `gen_hotkeys` (sent before 3025; the answer is stored without a check), `vis_nsfs` (unreachable, section 7) | binary [A for `in_flac` and `gen_hotkeys` send sites] |
| **1335** | 0 | Combined with the literal `genres.txt` by a wide path-combine routine. | Pointer to a NUL-terminated **wide directory path**. | `in_vorbis` | binary [A] |
| **3015** | **0** | Stored; later **called with no arguments**; the integer result is divided with a signed modulus (97 re-derived; others quoted by the draft as 21, 17, 101 [N]) to make an array index. A negative result would index below the array. | **Address of a function returning a non-negative int** (a random number). | `in_vorbis` (sent twice back to back, wparam 0 then 1, the very first thing after the window test), `vis_milk2` (wparam 0 only, in its module init: an answer of 0 or 1 is replaced by the plug-in's own generator, so 1 is tolerated there), `vis_nsfs` (unreachable) | measured (message sent by `in_vorbis`, answered 1) + binary [A for `in_vorbis` and `vis_milk2`] [C: `vis_milk2` was missing in the draft] |
| **3015** | **1** | Stored; called with no arguments; the result is taken from the **floating-point return register as a 32-bit float**. | **Address of a function returning a float.** Range unknown; the use (a table of 97 values filled, then differences) suggests a uniform value (reading: 0 to 1). | `in_vorbis` | binary [A] |
| **3025** | 0 | The answer is the service manager pointer. **The plug-ins treat the value 1 as "unsupported" and replace it by 0.** `gen_hotkeys` and `out_wave` then **test for 0** (and 1) and skip; `out_disk`, `dsp_sps`, `vis_avs`, `vis_milk2` call through it unchecked, so 1 is as fatal as 0 for them. | Pointer to the service manager. | `out_disk`, `dsp_sps`, `vis_avs`, `vis_milk2`, `vis_nsfs`, `gen_hotkeys`, `gen_jumpex`, `out_wave` | measured (sent by `out_disk`, answered 1, null read follows) + binary [A except `gen_jumpex`, `vis_nsfs` N] |

Other ids seen in code that start-up does not reach (not analysed): 3026, 365, 3032 and ids used between dialogs. (reading) [N]

The 3015 pair also means that the host must supply **real function addresses
valid inside the plug-in host process**. (reading)

## 5. Services looked up at start-up

Identifiers are the 16-byte values in the DLLs' data, written in the usual
8-4-4-4-12 text form. The role names are **readings** derived from the calls the
plug-ins make on them, except where stated.

| Identifier | Neutral role | Looked up by (start-up) | If the lookup returns 0 | Calls the plug-ins make (message, argument count) |
|---|---|---|---|---|
| 30AED4E5-EF10-4277-8D49-27AB5570E891 | **Language service** | `in_wave`, `in_flac`, `in_vorbis`, `out_disk`, `out_wave`, `dsp_sps`, `vis_avs`, `vis_milk2`, `vis_nsfs`, `gen_hotkeys`, `gen_jumpex` [A: identifier read in `in_wave`, `in_flac`, `in_vorbis`, `out_disk`, `out_wave`, `dsp_sps`, `vis_avs`, `vis_milk2`, `vis_nsfs`, `gen_hotkeys`, `gen_jumpex`] | **Fatal** for `in_wave` (re-derived: the start call goes through the stored null), and by the same pattern for the other fault cases; tolerated by `out_wave`, `gen_*` when they skip everything. | 30 (2 args), 10 (5 args, ANSI text), 11 (5 args, wide text), 91 (no arguments, `vis_milk2` getter; an answer of 0 is tolerated); dialog-creation calls only from configuration windows |
| AEFBF8BE-E0AA-4318-8CC1-4353410B64DC | **Configuration service** | `in_wave`, `in_flac`, `in_vorbis` | Tolerated at start-up (null-checked). | 10 (1 arg: a 16-byte group identifier, result = a group object), then on the group: get an int, a bool, a string by item name |
| 23B96771-09D7-46D3-9AE2-20DCEA6C86EA | **Application service** | `in_vorbis`, `out_wave`, `vis_milk2`, `gen_hotkeys`, `gen_jumpex` | Tolerated at start-up. | 365 (6 args, routes mouse-wheel messages in configuration dialogs), 360 (2 args, boolean); only from dialog code [N] |
| 000CF46E-4DF6-4A43-BBE7-40E7A3EA02ED | **Memory service** | `in_flac` | Tolerated at start-up; used unchecked when the album-art provider returns data. | 0 (1 arg: size; result = memory the host later releases) [N] |
| 57B7A1B6-700E-44FF-9CB0-70B92BAF3959 | not identified | `vis_milk2` | tolerated | none seen at start-up |
| C5618774-7177-43AA-9906-933C9F40EBDC, 83D6CD21-D67A-4326-A5B2-E1EFD664ADB5, A0099AA7-F980-45CF-818D-64EAA9F4EF4B, B8B8DA7C-1F35-4A6D-95FA-C7E9651D5DC0, 5F8D8373-EAA7-4390-B5AB-402E86A5F9DD | not identified | `gen_jumpex` (media-library and skin side, out of scope) | tolerated | not analysed; `gen_jumpex` loads when the manager is absent |

[C] The draft listed 858FBF71-9878-4D86-BFDD-8FEA8361238C (`vis_nsfs`),
250FAA3C-20CD-49DB-A932-67B1C0191B0E (`gen_hotkeys`) and
4693FA7D-2055-4B36-A239-0AD998B5A884 (`gen_jumpex`) as "not identified service
lookups". They are the **plug-ins' own language identifiers**: each equals the
entry 65535 of that plug-in's own string table, and is passed to the language
start call as the plug-in's own id. They are not services.

**The plug-ins' own identifiers.** Each plug-in's string-table entry 65535 holds
its own language identifier in text (for example the one in `out_disk`'s table is
1A710E67-5180-49AC-8102-105856ED0A2F); the 16-byte value is what the plug-in
passes as argument 2 of the language start call. `vis_milk2` passes
226275F6-3318-4D4B-A6B3-5B1B5B077BE8, which differs from the entry 65535 of its
table (C5D175F1-E4E4-47EE-B85C-4EDC6B026A35); why is unexplained. The
identifier B6CB4A7C-A8D0-4C55-8E60-9F7A7A23DA0F is a configuration group that
the input plug-ins read playback options from (replay-gain keys, a
"preferred only" flag appear as strings in `in_flac`). [A]

### 5.1 The language service in detail

All text is the plug-in's **own resource string table**. Entry 0 or 14 is a
product name (which one is used varies), 65534 (where present) is a description
template with one `%s`, 65535 is the language identifier text.

| Msg | Meaning | Arguments (in order) | Result | Start-up use |
|---|---|---|---|---|
| 30 | "Start language support" | 1: the plug-in's module handle; 2: the plug-in's own 16-byte identifier | an opaque **language handle** (any non-zero value works as far as the plug-ins show; they hand it back as argument 1 of the string calls) | once per plug-in, stored without a test [A for `in_wave`, `dsp_sps`, `vis_milk2`] |
| 11 | Get a wide string | 5: language handle, module handle, string id, destination buffer or null, buffer length | pointer to the NUL-terminated wide string. With a **null buffer** the plug-in uses the returned pointer **as the format string of a wide printf** (id 65534). | `in_wave`, `in_flac`, `in_vorbis`, `out_disk`, `out_wave`, `gen_*`. **A null or garbage result is fatal** (the C runtime's invalid-parameter path ends the process); an empty valid string is not. [A for `in_wave`; the "fatal" statement N] |
| 10 | Get an ANSI string | 5: same, with a buffer and its size | the buffer pointer | `vis_avs` asks **id 0** (buffer of 128) and `dsp_sps` asks **id 14** (buffer of 256) and print `"%s v2.82"` / `"%s v0.39b"` style descriptions; a null result prints "(null)" and does not fault. [C: the draft said ids 65534 and 0 are all that is needed; `dsp_sps` has no entry 65534 and asks 14] |

Observable contract: **load the string with the given id from the string table
of the module handle passed as argument 2**, copy it into the buffer if one was
given, return a pointer to it; for a null buffer return a pointer that stays
valid at least until the printf has run. The ids asked at start-up are the
product-name entries (0 or 14) and 65534 where the table has it. (reading from
the argument roles; the argument order itself is audited [A] in the wrapper of
`in_wave`.)

### 5.2 Message numbers differ by build

The later dialog-creation calls carry different numbers in the May 2013 build of
`dsp_sps` (50/51) and in the November 2013 builds (53/54) [N]. The numbers needed
at start-up (10, 11, 30, 50, 300, 310, 20) were identical in every plug-in the
auditor looked at (`in_wave`, `in_flac`, `in_vorbis`, `out_disk`, `out_wave`,
`dsp_sps`, `vis_avs`, `vis_milk2`) [A for those; N for the rest].

## 6. Module-struct fields a 5.x host fills

### 6.1 Input module (S09 section 3)

| Offset | Member | 2.x host | 5.x host (as the plug-ins expect it) | Evidence |
|---|---|---|---|---|
| 0x00 | version | 0x100 | The three 5.x plug-ins carry **0xFF000101**. | binary [A] |
| 0x04 | description | static ANSI | Static ANSI literal in the file ("nullsoft(in_xxx.dll)"), **rewritten by init to a wide string** (up to 256 characters, built from string 65534 and a version text). Read it **after init**, as wide text. | binary [A for `in_wave`, `in_vorbis`] |
| 0x08, 0x0C | main window, module handle | host fills | Init tests the window first and returns **1 (failure)** when it is not a window, 0 on success. | binary [A] |
| 0x10 | file extensions | static | [C] `in_wave` and `in_vorbis` carry a null pointer in the file; `in_flac` carries a static text ("FLAC"). The plug-ins rewrite the member from their settings section (334 or 1334 needed first). Read it after init. Whether `in_vorbis` does so inside init is not established. | binary [A for the file values; N for the timing] |
| 0x90, 0x94 | set-info, output module | host fills | as 2.x | binary [N] |
| **0x98** | **service manager pointer** | member does not exist | **Host fills before init.** The plug-in does not test it before the registration call. Every service call of the three input plug-ins reads it from the module's own memory. Static value in the file is 0. | binary [A: the global at RVA 0x60D8 / 0xC160 / 0x35510 in `in_wave` / `in_flac` / `in_vorbis` is module + 0x98, is never written by plug-in code and is read at every service call] |

Module addresses (RVA): `in_wave` 0x6040, `in_flac` 0xC0C8, `in_vorbis`
0x35478. The three modules sit in the plug-in's writable data, so a host that
writes 4 bytes at offset 0x98 writes into the plug-in's own variable. No member
beyond 0x98 is read. [A]

### 6.2 Output module (S09 section 4)

No new member. Version 0x11 (wide description). The description pointer is
**null in the file** (`out_disk`) and set by init **only after** the language
lookup worked; a host that reads it before init, or whose init took the
"unsupported" path, sees null/empty. The survey shows this: `out_wave` loaded as
the empty text. Module handle at 0x10 and main window at 0x0C are read in init.
`out_wave` looks for a module named in_wm.dll in the process; with none it
returns its own module (not a host duty). binary [A for `out_wave` and `out_disk`] + measured

### 6.3 General-purpose module (S09 section 7)

Version 0x11, static ANSI description. Init rewrites it only when 3025 gave a
manager (`gen_hotkeys`, `gen_jumpex`). Without one the literal survives (matches
the survey). No new member. binary [A for `gen_hotkeys`; N for `gen_jumpex`] + measured

### 6.4 DSP header and module (S09 section 6)

Header version **0x21**, description null in the file and set inside the
**getter** (only when the getter's argument is a valid window and the language
service worked). The getter takes **one argument, the main window**; it tests it
with a window-validity call and, when that fails, returns the static header
with the description unset. Module: parent window at 0x04; the first thing init
does is send 336 to the parent window. binary [A]

### 6.5 Visualization header and module (S09 section 5)

| Plug-in | Header version | Getter argument | Description |
|---|---|---|---|
| `vis_avs` | 0x101 [A] | main window; 3025 is sent to it first thing | null in file, set by the getter (ANSI) [A] |
| `vis_milk2` | 0x102 [A] | main window; 3025 is sent to it first thing | static ANSI "MilkDrop v2.25c" [N] |
| `vis_nsfs` | 0x101 [N] | main window | refuses on Windows Vista or later (section 7) [A] |

The module struct (S09) is unchanged. [N]

**The getter's argument is the 5.x-only fact.** The getters examined (`vis_avs`,
`vis_milk2`, `vis_nsfs`, `dsp_sps`) end with a plain return, i.e. the caller
removes the arguments, so a host that passes the main window to every getter
cannot harm a plug-in that ignores it. [A] The current host declares the getters
without an argument; the plug-ins then read whatever lies on the stack. [A,
clean tree read-only]

## 7. Per plug-in: what is read through null, where it comes from, the least to start

Fault sites are the survey's (measured); RVA = measured address minus the DLL's
preferred base. [A: every row's fault address was re-derived against the code]

| Plug-in | Measured fault | RVA | What is dereferenced | Source of the null | The least that lets it start |
|---|---|---|---|---|---|
| `in_wave` | init, read 0x0 | 0x2C3D | The manager's table, in the "register a factory" call (message 10). | Module member 0x98. Init faults before sending any IPC; the survey saw none. | Fill 0x98 with a manager (section 3) with a language service; answer 334 (ANSI) afterwards. |
| `in_flac` | init, read 0x0 | 0x5578 | Same call, same reason. | Member 0x98. | As `in_wave`, plus 1334 (wide) after the registration. |
| `in_vorbis` | init, read 0x0 | 0xC4E2 | Same call. | Member 0x98 (checked for the three lookups, not for the registration). 3015 was sent twice before and the answers (value 1) stored as function pointers. | Fill 0x98; answer 3015/0 and 3015/1 with real function addresses; 334 and 1335 afterwards. |
| `out_disk` | init, read 0x0 | 0x2FC4 | The manager's table. | The 3025 answer 1, converted to 0 by the plug-in and used unchecked. It sent 334 first, then 3025. | A valid manager as the 3025 answer, with the language service. |
| `dsp_sps` | init, read **0x1** inside the system's formatting routine | call site RVA 0x38B0 | The answer to **336**, used as a string pointer for `%s`. | Host answered 1 to 336. The getter also received an invalid window and skipped its lookups. | 336 = writable directory path; getter called with the main window; valid manager with the language service. |
| `vis_avs` | header getter, read 0x0 | 0x486D3 | The manager's table, in the language lookup. | The getter's argument is not the main window, so 3025 went to a dead window and returned 0. No message appears in the trace (messages to the real main window of the other plug-ins do). | Getter argument = main window; 3025 = manager with the language service. |
| `vis_milk2` | header getter, read 0x0 | 0x34404 | Same, first lookup. | Same. (It also needs the DirectX 9 runtime in the guest, an unrelated finding of the survey notes [N].) | As `vis_avs`. |
| `vis_nsfs` | header getter returns null | - | nothing: **the plug-in's own operating-system gate**; it returns no module when the reported major version is 6 or higher. | Not host related. | Cannot start on a current Windows without a compatibility shim. [A] |
| `gen_hotkeys`, `gen_jumpex`, `out_wave` | loaded | - | They send 3025 (`gen_hotkeys` sends 1334 first, `out_wave` sends 334 in a helper), compare with 1 and 0 and skip all service use. | n/a | Nothing; with a manager they take the full path, **not yet measured**. [A for `gen_hotkeys`, `out_wave`; N for `gen_jumpex`] |

### 7.1 Order inside the three input plug-ins' init

Window test; read the manager from member 0x98; look up the configuration,
memory (or application) and language services, each stored only when non-null;
register their own factories; start language support through the language
service; build the description; send 334/1334/1335; read their settings section.
`in_vorbis` sends the two 3015 requests before everything else. The three
measured fault addresses are each the first read of the manager's table in the
registration helper. [A for `in_wave` and `in_vorbis`; N for `in_flac`'s order]

## 8. The least a 5.x host must provide, per plug-in (checklist)

| Plug-in | Pass window to getter | Manager at 0x98 | 3025 manager | Language service | 334 | 336 | 1334 | 1335 | 3015/0 | 3015/1 |
|---|---|---|---|---|---|---|---|---|---|---|
| `in_wave` | - | yes | - | yes | yes | - | - | - | - | - |
| `in_flac` | - | yes | - | yes | - | - | yes | - | - | - |
| `in_vorbis` | - | yes | - | yes | yes | - | - | yes | yes | yes |
| `out_disk` | - | - | yes | yes | yes | - | - | - | - | - |
| `dsp_sps` | yes | - | yes | yes | yes | yes | - | - | - | - |
| `vis_avs` | yes | - | yes | yes | yes (module init) | - | - | - | - | - |
| `vis_milk2` | yes | - | yes | yes | not checked | - | - | - | optional (0 or 1 tolerated) | - |
| `vis_nsfs` | (unreachable) | - | - | - | - | - | - | - | - | - |
| `gen_hotkeys` | - | - | optional | optional | - | - | give a valid wide path | - | - | - |
| `gen_jumpex` | - | - | optional | optional | - | - | - | - | - | - |
| `out_wave` | - | - | optional | optional | give a valid path | - | - | - | - | - |

"yes" = needed to get through the start-up call without a fault; "optional" =
the plug-in skips the feature without it. [C: `vis_avs` 334, `vis_milk2` 3015
and the `gen_hotkeys` 1334 cell were added or changed; `gen_hotkeys` stores the
1334 answer unchecked, so a bad pointer may fault later - not measured.] The
matrix is a **binary** result for the columns and a **reading** for the claim
that nothing else is needed: the later steps of each init were followed to the
settings reads, no further.

## 9. What the 2.95 reference does, and the separation rule

| Request | 2.95 reference (S10) | 5.x extension (this spec) |
|---|---|---|
| 3015, 3025 | 1 | 3025: manager pointer; 3015: two function addresses |
| 334, 335, 336 | never probed (S10) | strings as in section 4 |
| Input module offset 0x98 | does not exist (38 members) | filled only when the version value is 0xFF000101 (the value carrying the 0xF0000000 and 0x0F000000 bit groups) |
| Getter argument | none | main window |

The two sets never meet: a 2.x plug-in sends none of the 5.x requests and
ignores the extra getter argument. (reading)

## 10. What this spec does not describe

* Services of the media library, the skin engine or the general-purpose
  framework window: `gen_jumpex` looks up several identifiers on that side and
  tolerates their absence.
* The factories the plug-ins register and the interfaces they implement.
* The configuration dialogs' use of the language and application services.

## 11. Open questions

| # | Question | What would settle it |
|---|---|---|
| 1 | Does a host that gives the 5.x answers let the seven start? Nothing was measured with such a host. | A host variant with: member 0x98 filled, the answers of section 4, a language service answering 30, 10 and 11, and a trace of every generic call (message and argument count). |
| 2 | The host passes no valid window to the getters: the current host declares them without an argument [A]; the silent trace of `vis_avs` agrees [A]. A direct trace of the value was not made. | Pass the main window; the survey then shows the getters' messages. |
| 3 | Range of the float generator (3015/1) and whether the integer generator should cover 15 or 31 bits. | A measurement of the real 5.66 player in the VM (owner's go-ahead). |
| 4 | What a real 5.x host does with a flagged input plug-in that has no manager. Probably never happens. | Same reference run. |
| 5 | Whether `vis_nsfs` passes under a compatibility setting that reports an older Windows version. | A VM run under such a shim; low value. |
| 6 | `gen_hotkeys`, `gen_jumpex`, `out_wave` were only observed "loaded"; the survey prints a trace only for failures. | The survey with the trace printed for successes. |
| 7 | Identity of the unidentified service identifiers (section 5). Not needed to start the seven. | Not worth doing unless a plug-in needs one. |
| 8 | Dialog-creation numbers differ between builds. | Only if dialogs are implemented. |
| 9 | Who frees memory the album-art provider returns. | Out of scope for start-up. |
| 10 | Why `vis_milk2` passes a language identifier different from its table entry 65535, and what its message 91 asks. | A trace of the generic call. |
