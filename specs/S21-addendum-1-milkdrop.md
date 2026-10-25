---
Spec ID: S-21 addendum 1
Title: What MilkDrop 2.25c (vis_milk2.dll) asks of its host, from the end of the start-up checks through its first rendered frame
Sources consulted (category only): dirty room: static analysis (byte parsing, a 32-bit x86 disassembler, a decompiler used as a reading aid; nothing executed) of `vis_milk2.dll` from the Winamp 5.66 installer, of the other plug-in DLLs of that folder for the service-message numbers, and of the export and import tables of the two d3dx9 libraries staged next to it; the installer's file listing (names and sizes only; nothing extracted or run); dirty room: the report of the VM test "MilkDrop in the host" (run of 2026-10-25 16:44, the only runtime evidence; no new VM run was made); clean repo, read-only context: audited S09 (module layout), audited S21 and its implementation notes, and the host's plug-in loader (library search path only)
Author agent: dirty-room spec agent, lane S21 (RFC 0021), addendum task
Auditor: spec auditor subagent (lane S21)
Audit date: 2026-10-25
Dirty-room authorisation: RFC 0021 - the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: the 5.66 build of the plug-in (product string "MilkDrop v2.25c", visualization header version 0x102, SHA-256 2d27437afe4f161539b935e578c56d4d75b5bedc716e4a9a69984f88daf47c5e)
Status: audited (with changes; see the audit note S21-addendum-1-audit.md in the lane folder)
---

# S21 addendum 1 - MilkDrop 2.25c: start-up after the checks, up to the first frame

## 0. How to read this addendum

This addendum continues the audited S21. Everything said there (the generic
call, the service manager, the language service, the 3025 / 334 / 336 / 1334 /
1335 / 3015 answers) stays as it is, with the corrections of section 2.5.

No plug-in was run on the host. The only runtime evidence is the VM report of
the test that loads `vis_milk2.dll` in the plug-in host and feeds it frames;
it ends at the dialog "Error loading main menu". Everything after that point is
a reading of the binary and says so.

Evidence tags. The letter in brackets is the auditor's mark.

| Tag | Meaning |
|---|---|
| **measured** | Seen at run time in the VM report (trace order, dialog text, return code). |
| **binary** | Read from the DLL's bytes: static data, resources, import table, the path the code takes. |
| **reading** | A hypothesis built on the above, or a statement about the operating system that the binary does not show; an implementer must not rely on it without a measurement. |
| [A] | The auditor derived the statement a second time, independently, from the bytes. |
| [N] | Not derived a second time by the auditor; it stands as the draft wrote it. |
| [C] | Corrected by the auditor; the text here is the corrected one. |

Addresses are RVAs (address minus the preferred load address 0x7ED0000), for
example `RVA 0x2E577`. They locate a place in the code so that a later checker
can find it; no listing is reproduced and no state variable is named by address.

### Names used in this addendum

S21's neutral names are kept (service manager, service factory, service object,
generic call, language service, configuration service, application service,
memory service, main window, IPC n). New neutral names:

| Name here | Meaning |
|---|---|
| menu-load call | Language service message 52 (section 2). |
| language handle | The value the language service returns for message 30. The plug-in keeps it and uses it **as a module handle** to look up its own text resources (section 2.5). |
| own module | The plug-in's own module handle: the allocation base of the module that holds the getter. It is also what the plug-in passes as argument 1 of message 30 and as the "module handle" argument of the string calls. It should equal the `dll_instance` member the host stores in the module record (S09). |
| plug-in object | The one global object that holds the plug-in's state (a structure of several megabytes). |
| plug-in window | The top-level window of class "MilkDrop2" that the plug-in creates for itself. |
| device | The Direct3D 9 device the plug-in creates for that window. |
| plug-ins folder | The directory that IPC 1336 returns, or, when that returns 0 or 1, the directory of the module whose handle is in the `dll_instance` member of the module record (section 4.2). |
| settings folder | The directory that IPC 1335 returns. |
| data folder | `<plug-ins folder>\Milkdrop2\data\`. |

## 1. Answers in one page

| # | Question | Answer | Evidence |
|---|---|---|---|
| 1 | What is language message 52? | The call that **loads a menu resource**. Arguments in order (each passed by address, S21 section 3): **1** the language handle, **2** the own module, **3** the resource identifier (the plug-in passes **110**). It wants back, in the result cell, a **menu handle** (a real menu of the plug-in-host process) and a nonzero return. The plug-in then asks that menu for its **first sub-menu** (position 0); both must be non-null, otherwise the plug-in shows "Error loading main menu" (menu null) or "Error loading context menu" (no first sub-menu) and its `init` returns 1. | binary [A]; measured (the dialog and the call with 3 arguments) |
| 2 | The least a host returns | Return 1 and write a menu handle whose position 0 is a pop-up. The natural and sufficient implementation: load menu resource 110 from the module of **argument 2** (the own module) and **ignore argument 1** (or make message 30 answer the same module handle, so that the two agree). The plug-in **owns** the handle afterwards and destroys it at shutdown; give it a fresh handle it may destroy. | binary [A] |
| 3 | What else does MilkDrop ask of the host before the first frame? | Section 4 (window messages), 5 (services), 6 (files), 7 (windows), 8 (DirectX). In one line: IPC 0, 3015/0, 104 (only when the reported version is below 0x4000), 1336, 1335, 3044, 505 (answer 0 = "no embedding"), 611 (twice, create and destroy); the language messages 30, 10, 11, 52 and (getter) 91; **eight** data files `data\*.fx` ([C], not seven): only `include.fx` stops start-up when missing, each of the other seven shows a blocking dialog and start-up goes on, but all eight are needed in practice once pixel shaders are in use; a Direct3D 9 device of device type 1 must be creatable; a **d3dx9 library** numbered 24 to 36 must be loadable by name (the plug-in's folder is searched by the current host), or IPC 3044 must hand the plug-in a module. Per frame: IPC 125 and IPC 213 (current title). | mixed, see the sections |
| 4 | What does the render step need from the data arrays? | Only the **two waveform rows** (576 signed bytes each, 0 = silence); the spectrum rows are not requested (spectrum row count 0 in the module). Nothing is validated. All-zero data is accepted and gives "neutral" relative levels (1.0) in the plug-in's own analysis; constant data behaves like a steady signal after a short transient (section 9). | binary [A] for the layout, the row count and the zero guard; reading for the constant case |
| 5 | Timers, threads, run for minutes? | **No timers** (no timer API is imported). **One worker thread** (a scan of the preset folder), started during allocation, ending by itself; the plug-in's `quit` stops or terminates it. Side effects the host must know about: `timeBeginPeriod(1)` for the whole run; the plug-in toggles the machine's **Scroll Lock** state with synthetic key events; it takes the **foreground focus back to the main window** at quit. A user closing the plug-in window ends in a **quit message on the creating thread**. | binary [A] |
| 6 | Can it draw on a software renderer? | **Yes in principle, not proven.** The plug-in accepts any device of device type 1 (whether the Windows basic render driver reports as that type is an operating-system fact, reading); it needs no particular pixel-shader version (without pixel shaders it falls back to the fixed-function "MilkDrop 1" mode); the VM already got through adapter query, capability query and device creation (measured by trace order, section 8.1). Not known: the pixel-shader version the guest reports and whether anything reaches the screen. | binary [A] + measured; drawing open |

## 2. Language-service message 52

### 2.1 The call, argument by argument

| Item | Fact | Evidence |
|---|---|---|
| Caller | One call site, in the routine at RVA 0x2E4D0 (create the GDI fonts, then load the menu), call at RVA 0x2E577. That routine runs after the Direct3D device exists (section 3). | binary [A] |
| Message / count | 52, three arguments. Same calling convention as every language-service call (S21 section 3: object in ECX, four stack words: message, result cell, argument array, count; callee pops 16 bytes). | binary [A]; measured ("Language message 52 with 3 argument(s)") |
| Argument 1 | the **language handle** (the value the host returned for message 30; the plug-in keeps it in a global) | binary [A] |
| Argument 2 | the **own module** | binary [A] |
| Argument 3 | the integer **110** (resource identifier). A menu resource with this id exists in the plug-in's own resources; it is the only menu resource there. | binary [A] |
| Return | nonzero = understood. If 0, the plug-in uses 0 as the menu. | binary [A] |
| Result cell | the **menu handle**, kept by the plug-in | binary [A] |

### 2.2 What the plug-in does with the result

| Step | Fact | Evidence |
|---|---|---|
| 1 | If the handle is 0: message box (string 445, "Error loading main menu"), the routine returns 0, and the master start-up step fails, so `init` returns 1 (refused). This is exactly the VM run's end state. | binary [A]; measured |
| 2 | Otherwise `GetSubMenu(menu, 0)` (RVA 0x2E5D4) is kept. If 0 (the handle was not a menu, or its first item is not a pop-up): message box (string 446, "Error loading context menu"), same failure. | binary [A] |
| 3 | The sub-menu is the **context menu** of the plug-in window: shown with `TrackPopupMenuEx` on right-click in windowed mode (window procedure, RVA 0x31FD8; the call at RVA 0x325D4 uses the stored sub-menu). The plug-in toggles check marks on it with `CheckMenuItem` (ids 40008 "Show Help", 40009 "Show Playlist"; RVA 0x2F37D and 0x2F3AB). | binary [A] |
| 4 | At shutdown (RVA 0x2FF0D) the plug-in calls `DestroyMenu` on the handle it was given (the top-level one, not the sub-menu) and forgets it. The handle must therefore be one the plug-in may destroy, and it must not be shared or reused. A second `init` asks again. | binary [A] |

### 2.3 The menu resource the real build carries

Resource type MENU (classic template, not the extended one), id 110, one
top-level pop-up ("Options:") holding five commands and two separators: 40007
Fullscreen (Alt+Enter), 40010 Desktop Mode (Alt+D), separator, 40008 Show Help
(F1), 40009 Show Playlist (P), separator, 40006 Quit. The command ids reach the
plug-in window as menu-command messages when the user chooses an item. A host
that **builds a replacement menu** instead of loading resource 110 needs: a
first item that is a pop-up; items with these ids if the commands are to work
(they are not needed to get through start-up).

| Evidence | binary [A] (the resource was parsed; the plug-in's use of position 0 re-derived) |
|---|---|

### 2.4 What the least implementation is (behaviour, not code)

"Load the menu resource whose identifier is in argument 3 from the module of
argument 2, write the resulting menu handle into the result cell and return 1;
if the resource does not exist return 0." Argument 1 carries no information the
host needs when message 30 answered the module handle the plug-in passed in
(section 2.5 C1); if message 30 answered something else, **ignore argument 1**
- loading from it could pick up a resource 110 of the wrong module. Message 52
is the only three-argument call of this kind any plug-in of the folder sends;
53 and 54 are the dialog-creation calls (S21 5.2). [A: a byte scan of all
plug-in DLLs of the folder for the constant-message pattern found 52 only in
`vis_milk2`; `gen_jumpex` (53, 54), `in_vorbis` (53) and `vis_nsfs` (54) use
the dialog calls. The scan only finds a message number that is pushed as an
immediate before the call.]

### 2.5 Corrections to the audited S21

| # | S21 said | Correct for `vis_milk2` | Evidence |
|---|---|---|---|
| C1 | Section 5.1, message 30: the result is "an opaque language handle (any non-zero value works as far as the plug-ins show)". | The plug-in uses the value as a **module handle**. Its text-resource finder (RT "TEXT" resources, routine RVA 0x34383) first looks for resource 130 **only** in the language handle (no fallback; absent in the real build, so it stays unused) and for resource 129 first in the language handle and, when that finds nothing, in the own module. A foreign value is therefore survivable (the VM run measured this: the plug-in passed that step and reached message 52), but the **safe return is the module handle the plug-in passed as argument 1 of message 30**: the plug-in's own resources are then the language. | binary [A]; measured (survived) |
| C2 | Section 5, table row for message 91: "no arguments, `vis_milk2` getter; an answer of 0 is tolerated". | Message 91 returns a **C-runtime locale object**; the plug-in passes it as the locale argument of its own locale-taking text-conversion calls (`_sscanf_l`, `_atoi_l`; fourteen places read the value; the ones inspected are in the configuration dialog, the preset-file parsing and the registry value read). A **0 answer means "use the current locale"**; `setlocale` is not imported, so the plug-in's own runtime stays in its default locale (reading: another plug-in of the same process could still change it). A non-zero answer must be an object **of the same C runtime as the plug-in (MSVCR90)**; one from another runtime would be misread. So: answer 0 (or "not understood"). | binary [A] (global stored in the getter; consumers read); reading for the 0 = current-locale rule, which is a documented C-runtime rule, not shown by the binary |
| C3 | Section 5, rows "not identified": 57B7A1B6-... (`vis_milk2`) "none seen at start-up". | The service 57B7A1B6-700E-44FF-9CB0-70B92BAF3959 is looked up in the getter and **called with message 30 and four arguments** (a four-byte tag 0x0075726C, the integer 10, a wide string pointer, a pointer to a flag byte) from the routine that opens a URL or document with the shell (RVA 0x33824, call RVA 0x3386D). Only a user action reaches it - and the **Yes button of the string-602 box** (section 4.3), which calls the same routine. The plug-in does **not test the object for null**: with a failed lookup, those actions dereference null. The application service (23B96771-...) is called only from the configuration dialogs (message 365) [N]. | binary [A] |

## 3. Order of events, from the end of the start-up checks to the first frame

"Step" numbers are used by the later tables. All of it runs on the thread that
calls `init` (the plug-in window is created there, section 7).

| Step | What the plug-in does | What the host sees / must provide | Evidence |
|---|---|---|---|
| 0 | The getter (RVA 0x3496F) and the module list are as in S21. The module record is at RVA 0x41DD8; static values (S09 offsets): latency 0 (0x14), delay 10 ms (0x18), spectrum rows 0 (0x1C), waveform rows 2 (0x20). The plug-in later **rewrites the latency member itself** while it runs (three store sites: 500, 8, or a computed number; RVA 0x31261, 0x3127C, 0x312A5), so a host must read it fresh when it picks the frame. | -- | binary [A] for the static values and the store sites; reading for the purpose of the stores |
| 1 | `init` (RVA 0x34B3B). First: IPC 0 once (cached) - the value is the host version. | version answer (see 4.1) | binary [A]; measured |
| 2 | IPC 3015 (wparam 0), once, stored as the plug-in's random-number function; an answer of 0 or 1 is replaced by the C runtime's `rand`. | S21 section 4 | binary [A]; measured |
| 3 | A one-second wait (50 ms steps) for the "previous instance finished" flag, which is 1 in the file; so it returns at once on a first start. | nothing | binary [A] |
| 4 | **If the reported version is below 0x4000:** IPC 104; the answer must be **1** (playing), otherwise a message box (strings 464/465, "no music") and `init` returns 1. With version 0x4000 or higher this step is skipped. | the player state; the test sets "playing" | binary [A]; measured (the first 104 of the trace) |
| 5 | Pre-initialisation (RVA 0x2F4BC): sets the built-in defaults; IPC 1336, IPC 1335; builds the folder and file names; creates `<settings folder>\Plugins\Milkdrop2\`; moves older files up into it; looks up the desktop windows ("Program Manager" and its list view); the later steps read the INI. | 4.2, section 6 | binary [A for 1336/1335 and the folders, N for the desktop lookup]; measured (1336 then 1335) |
| 6 | Window-class registration (class "MilkDrop2", `RegisterClassW`, RVA 0x2C316 and 0x2C425) and plug-in-object set-up. | nothing | binary [A] |
| 7 | **Direct3D context creation** (RVA 0x2C9D3, one long routine): load d3d9; `Direct3DCreate9`; IPC 3044 (RVA 0x341D6); adapter enumeration; capability query; IPC 505 (embedding attempt); `CreateWindowEx` for the plug-in window; **IPC 611 with the new window handle**; the window is shown (this causes a background-erase message that sends **IPC 104** from the window procedure); `CreateDevice` (RVA 0x2D84D). | 4.3-4.5, section 7, section 8 | binary [A]; measured (3044, 505, 611, 104 in this order) |
| 8 | Fonts and menu (RVA 0x2E4D0): six GDI fonts, then **message 52**. | section 2 | binary [A]; measured (stops here) |
| 9 | Data files (the second slot of the plug-in object's function table, RVA 0xD436): reads `data\include.fx`, then `warp_vs.fx`, `warp_ps.fx`, `comp_vs.fx`, `comp_ps.fx`, `blur_vs.fx`, `blur1_ps.fx`, `blur2_ps.fx` from the data folder. A file that cannot be opened shows a blocking message box (string 466, with the path). **Only `include.fx` also ends the step and makes `init` return 1** [C]; for each of the other seven the loader goes on after the dialog. | section 6 | binary [A] |
| 10 | Direct3D fonts (six, `D3DXCreateFontW`) and the main allocation (RVA 0xD7D3): picks the canvas size and the pixel-shader level, creates vertex declarations (only when the pixel-shader level is above 0 [C]) and render-target textures, compiles the fixed shader set when pixel shaders are in use, builds the warp mesh, starts the **preset-scan thread** and loads the first preset. | section 8, section 10 | binary [A for the order, N for the detail] |
| 11 | `init` returns 0. The host then calls `render` repeatedly (S09 5.3). | -- | binary [A] |
| 12 | Each `render` call: frame limiter (default 30 frames per second, spin or sleep), the analysis of section 9, **IPC 125 and IPC 213** (current title), drawing, `Present`. | 4.6 | binary [A for 125/213, N for the limiter] |

## 4. Window messages (0x400 to the main window)

"Main window" is the handle the plug-in got in the module record (the same one
the getter received). Ids are the lparam.

### 4.1 IPC 0 (version)

Asked once and cached (RVA 0x34363). The value steers three things: below
**0x4000** the plug-in asks IPC 104 at start (step 4) and, only if embedding is
in use, rounds the embedded window to a grid of 25 x 29 pixels; at **0x2900 or
higher** it asks IPC 505 (embedding) at all; below **0x5051** (only when
embedded) it shows the embed parent with the show-window call and gives it the
focus directly instead of sending the two user messages 0x466 / 0x467. A host
that keeps answering the 2.95 version (0x2950 class) is accepted: it makes step
4 required and makes the plug-in ask IPC 505. | binary [A] (all comparison
sites re-derived: the start-up check, the 505 gate, two grid-rounding sites,
two 0x5051 sites); measured (the version answered was below 0x4000: the first
104 is in the trace)

### 4.2 IPC 1336 and 1335 (folders)

| Id | Use of the answer | Needed | Evidence |
|---|---|---|---|
| 1336 | **0 or 1:** the plug-in uses the folder of the module whose handle it was given in the module record (`dll_instance`) - the folder of its own DLL when the host fills that member with the DLL's handle; **if the host leaves it 0 the folder of the host executable is used** [C, added]. Otherwise the wide string is formatted as "answer + backslash" and kept as the **plug-ins folder** (so: a wide directory path **without** a trailing backslash; with one, a doubled backslash results, which usually still works). All of `Milkdrop2\data`, `Milkdrop2\presets`, `Milkdrop2\textures` are looked up below it. The folder text is kept in a buffer of **260 wide characters** and built without a length check, so the path must stay well below that. | a wide path whose `Milkdrop2\data` sub-folder holds the eight `.fx` files; the pointer may be reused after the call (the plug-in copies it) | binary [A] |
| 1335 | **0 or 1:** `milk2.ini` is kept in the plug-ins folder. Otherwise the plug-in builds `<answer>\Plugins\Milkdrop2\milk2.ini` as its settings file, **creates** `<answer>\Plugins\Milkdrop2\` (one level only: `<answer>\Plugins` must already exist; the result of the creation is not checked) and, when an older `<answer>\Plugins\milk2.ini` exists and the new one does not, moves the old `milk2.ini`, `milk2_msg.ini`, `milk2_img.ini` and `milk2_adapters.txt` into it. | a wide path whose `Plugins` sub-folder exists; the pointer is used only during the call | binary [A] |

Note for the clean host (reading): S21's implementation notes make 1335 "a wide
directory path". MilkDrop appends `\Plugins\Milkdrop2\` to it, so the directory
the host gives must contain a `Plugins` folder; otherwise the plug-in works with
its defaults and never saves its settings (the audit did not check what the
host's current 1335 answer contains).

### 4.3 IPC 3044 (d3dx9 library)

Sent once per context creation, from the context-creation routine (RVA 0x2CA9D;
a second sender is the configuration dialog). wparam 0.

* The answer is taken as a **module handle of a loaded d3dx9 library** when it
  is neither 0 nor 1. The plug-in resolves exactly these twelve exports from it
  by name: `D3DXCreateFontW`, `D3DXMatrixMultiply`, `D3DXMatrixTranslation`,
  `D3DXMatrixScaling`, `D3DXMatrixRotationX`, `D3DXMatrixRotationY`,
  `D3DXMatrixRotationZ`, `D3DXCreateTextureFromFileExW`, `D3DXMatrixOrthoLH`,
  `D3DXCompileShader`, `D3DXMatrixLookAtLH`, `D3DXCreateTexture`; the results
  are stored and called later **without a test** (a library missing one of them
  crashes at first use).
* With **0 or 1** the plug-in calls `LoadLibraryA` with each of these names, in
  this order, and keeps the first that loads: `d3dx9_36.dll`, `_35`, `_34`,
  `_33`, `_32`, **`_31`**, `_30`, `_29`, `_28`, `_27`, `_26`, `_25`, `_24`.
  **Version 31 is tried (sixth); version 43 is not in the list.**
* If none loads, the message box of string 602 is shown (a yes/no question
  whose text says the plug-in "requires d3dx9_31.dll"), the context creation
  fails and `init` returns 1; the **Yes** button leads to the shell-open routine
  of section 2.5 C3. The text names 31 only as a wording of the plug-in; it is
  not a statement of which versions were tried, and it does not contradict the
  list above (reconciliation of the audit).
* By-name loading uses the host's library search. The current host sets the
  process's default search to the application directory, System32 and the
  folders added for approved plug-ins, and keeps the folder of each approved
  plug-in registered for the life of the process [N: read in the host's loader
  source, not tested]. A `d3dx9_31.dll` staged **next to `vis_milk2.dll`** is
  therefore found, after 36 to 32 failed; a staged `d3dx9_43.dll` is **never**
  found by name. Both staged libraries export all twelve names (verified in the
  export tables); `d3dx9_43.dll` contains a reference to the name of a separate
  shader-compiler library, which the process would then also have to find
  (reading); `d3dx9_31.dll` has none.
* The VM run passed this step (3044 is followed by 505 and 611 in the trace; the
  host answered 3044 with no 5.x answer). Which library loaded is **not
  measured**; with only `_31` and `_43` staged in the guest's plug-in folder,
  `d3dx9_31.dll` is the likely one (reading). A run that showed the string-602
  box had no d3dx9 numbered 24 to 36 findable by name.

Evidence: binary [A] (list order, the twelve names, the unchecked use, the
conditions on the answer, the message box); measured (message sent, run
passed); the host's search behaviour [N].

### 4.4 IPC 505 (embedding interface)

Asked **only** when all three hold: the embedding setting is on (INI key `skin`,
default 1 [A]), the screen mode is windowed (the embedding flag is cleared for
every other mode), and the reported version is 0x2900 or higher. wparam 0. The
answer is a **function address** (the host's embedding entry) or 0. With 0 the
plug-in clears its "embedded" flag and creates an ordinary top-level window.
With a nonzero answer the plug-in calls it, **with one pointer argument and
without expecting the callee to remove it (cdecl)**: a zero-initialised
window-state record of 0x11C bytes in which the plug-in has set a flags word
(0xA), a 16-byte identifier and a default rectangle taken from the INI; it uses
the returned window handle as the parent and sets that window's title. (An
answer of 1 would be called as a function: give 0.) | binary [A]; measured
(answered 0; the plug-in continued with a top-level window)

### 4.5 IPC 611 (vis command window)

wparam = the plug-in window handle, sent right after the window was created
(RVA 0x2D0D4), to the main window; wparam 0 (clear) is sent when the plug-in
destroys its window (RVA 0x2DA1A). The answer is not used. | binary [A];
measured (the first 611 with wparam 0x300B4)

### 4.6 Messages sent while running

| Id | wparam | When | Use | Needed | Evidence |
|---|---|---|---|---|---|
| 125 | 0 | every frame the plug-in's per-frame routine runs with its "normal frame" argument (RVA 0x256FF, called from RVA 0x11790) | current playlist position | an int | binary [A] |
| 213 | the position from 125 | directly after it, every such frame | **wide title string pointer**; copied with a length limit of 1023 characters and compared with the previous title (kept with a limit of 512); a change starts the song-title animation (when enabled, the default) | a pointer to a wide string, valid at least until the call returns; a null or 1 is survived by the length-limited copy routine (reading: operating-system behaviour) | binary [A]; reading for the null/1 case |
| 104 | 0 | from the plug-in window's background-erase handler (RVA 0x320AE; several other places for keys and drags) | "is playing"; when not 1 the plug-in presents the device without redrawing | 1/3/0 per S10 | binary [A]; measured (second 104) |
| 105 | 0 and 1 | only if the on-screen time display is switched on (off by default) | output time | S10 | binary [N] |
| 124, 212, 121, 250, 631, 277, 3033... | -- | user actions (playlist browsing keys, shuffle, mode switches, config dialog) | -- | S10 | binary [N] |
| menu-command / key posts | key and menu ids (for example 0x9C6D, 0x9C7A, 0x9CD0) | the plug-in forwards its keystrokes to the main window as Winamp commands, **posted** | player commands | the host may ignore them | binary [N] |

### 4.7 Not sent in the default settings

IPC 501 (video playing) is asked only when "start fullscreen" is set; IPC 3025
is sent from the getter only (S21). | binary [A for 3025, N for 501]

## 5. Services

All lookups use message 50 on the service manager (S21 section 3). In the getter,
in this order: the language service, 23B96771-09D7-46D3-9AE2-20DCEA6C86EA
(application service), 57B7A1B6-700E-44FF-9CB0-70B92BAF3959 (section 2.5 C3);
then message 30 (start language support) with the own module and the
identifier 226275F6-3318-4D4B-A6B3-5B1B5B077BE8; then **message 10** (ANSI string,
id 642, a buffer of 8 characters: the plug-in's string 642 is the set of "yes"
answer keys); then **message 91**. The language service is mandatory: its
object is used without a null test (message 30 in the getter itself). [A]

| Language message | Arguments | When | Needed | Evidence |
|---|---|---|---|---|
| 30 | own module, own 16-byte identifier | getter, once | the module handle for the resources (section 2.5 C1) | binary [A] |
| 10 | language handle, own module, id, buffer, length | getter (id 642) | copy the string, return the buffer | binary [A] |
| 11 | same, wide | every message box (captions: id 14 with a 64-character buffer; text calls pass **no buffer and length 0**, i.e. they expect a pointer to the string back) and every on-screen text | pointer to a valid wide string; **never null** (the plug-in passes it to `MessageBoxW` and to its own formatter); the plug-in's string ids run up to 466 plus 642, 4096.. and 65535 (language identifier) | binary [A]; measured (two calls before the dialog) |
| 52 | section 2 | RVA 0x2E577 | section 2 | binary [A] |
| 53, 54 | dialog creation (modeless / modal) | configuration dialog (ALT+K), the VJ-mode and help dialogs | not needed to draw | binary [N] |
| 91 | none | getter | 0 | section 2.5 C2 |
| 365 (application service) | six arguments, mouse-wheel routing | configuration dialog only | not needed | binary [N] |

String identifiers asked at start-up (all present in the plug-in's own table):
14 (caption), 19, 445 and 446 (the two menu errors), 444 (fonts), 466 (data
file), 464/465 (no music), 602, plus the per-failure messages of section 11. No
string is needed beyond the plug-in's own table. [N]

## 6. Files and registry keys

### 6.1 The data files init needs (exact list)

Folder: `<plug-ins folder>\Milkdrop2\data\` (section 4.2 for the plug-ins
folder; in the real installer `<plug-ins folder>` is the player's `Plugins`
folder). Read with the C runtime in binary mode, one file at a time, in this
order, each into a fixed buffer (32764 bytes for `include.fx`, 32768 for the
others; longer files are cut without a message). Sizes are those of the 5.66
installer, so all are far below the limits.

| # | File (relative to the data folder) | Size in the installer | If it cannot be opened |
|---|---|---|---|
| 1 | `include.fx` | 4905 | dialog (string 466) **and** start-up fails (`init` returns 1) |
| 2 | `warp_vs.fx` | 529 | dialog; start-up continues with an empty text |
| 3 | `warp_ps.fx` | 677 | same |
| 4 | `comp_vs.fx` | 533 | same |
| 5 | `comp_ps.fx` | 585 | same |
| 6 | `blur_vs.fx` | 366 | same |
| 7 | `blur1_ps.fx` | 2203 | same |
| 8 | `blur2_ps.fx` | 1911 | same |

Binary [A] (names, order, loader logic, buffer limits); the sizes are from the
installer's file listing. [C: the draft said seven files and that any missing
file stops start-up.] The effect of an empty text for files 2 to 8 later: with
pixel shaders in use (level 2 and up, section 8.2) the built-in shader set is
compiled from these texts, so an empty text is expected to make the compile
fail and `init` return 1; at pixel-shader level 0 they are not used (reading).

### 6.2 Other files and keys

| Item | Access | When | Effect if absent | Evidence |
|---|---|---|---|---|
| `<plug-ins folder>\Milkdrop2\presets\*.milk` (or the folder named by the INI key `szPresetDir`) | enumerate and read | by the scan thread, from step 10 | an on-screen "no preset files" text; the plug-in still runs on its built-in default state | binary [N for the absence behaviour] |
| `<plug-ins folder>\Milkdrop2\textures\*.*` | enumerated at preset load; read through `D3DXCreateTextureFromFileExW` | when a preset names a texture | "could not load texture" warning on screen; a missing folder makes the enumeration return "nothing" | binary [A for the enumeration pattern, N for the rest] |
| `<settings folder>\Plugins\Milkdrop2\milk2.ini` (or `<plug-ins folder>\milk2.ini`) | read at start; written (window rectangle keys at once in windowed mode; the settings block when options change) | step 5/7 | defaults | binary [A for read, N for the write timing] |
| `milk2_msg.ini`, `milk2_img.ini` (same folder) | read (custom messages, sprites) | on use | none | binary [N] |
| `milk2_adapters.txt` | **written** by the configuration dialog only | ALT+K | none | binary [N] |
| Registry: `HKCU\Control Panel\Desktop\WindowMetrics`, value "Shell Icon Size" | read (the only registry access, one call site) | desktop mode only | none | binary [A] |
| `Milkdrop2\data\vms_desktop.dll` | delay-loaded by its plain name, and loaded from the data folder by the plug-in | desktop mode only | none | binary [A] |

No other registry access, no network access. INI settings the plug-in reads
that change the paths above: `start_fullscreen` (default 0), `start_desktop`,
`fake_fullscreen_mode`, `skin` (default 1, embedded), `vj_mode` (default 0),
`MaxPSVersion` (default -1, automatic), `nTexBitsPerCh` (default 8),
`nTexSize`, `nMeshSize` (defaults give a 48 x 36 mesh). | binary [A for `skin`
and `MaxPSVersion`, N for the others]

## 7. Windows, dialogs and input side effects

| Item | Fact | Evidence |
|---|---|---|
| **Plug-in window** | Window class "MilkDrop2" (registered with `RegisterClassW`, style bits redraw-on-resize + double-click, black stock background brush, arrow cursor, icon resource 120 group, window procedure at RVA 0x31F94 which forwards to RVA 0x31FD8). Created with `CreateWindowExA` (class passed as the registered atom) on the `init` thread, title "MilkDrop 2", style **overlapped window with caption, system menu, sizing frame, minimize and maximize boxes** (0xCF0000, **not** visible on creation) in windowed mode, **parent = none** when IPC 505 returned 0, initial size 256 x 256 at (0,0); then `SetWindowPos` to the saved or default client rectangle and **shown** in the same call. Default size: a square whose side is a multiple of 64 pixels, at least 64 (derived from the monitor size; the saved rectangle of the INI wins). | binary [A for the registration, creation, style and size; N for the show call and the default-size formula] |
| Screen mode | Mode numbers in the plug-in: 0 fullscreen, **1 windowed**, 2 fake fullscreen, 3 desktop. Default (no INI): **windowed**, embedded requested (answered 0 -> top-level). `start_fullscreen=1` picks 0 (or 2 when IPC 501 reports video playing); `start_desktop=1` picks 3. | binary [N]; the window style choice per mode [A] |
| Rendering and focus | In windowed mode **every** `render` call draws; focus is not a condition (the focus test applies only to mode 0; the two other "skip this frame" flags are only ever set to 0). | binary [A] |
| VJ window | Class "MilkDrop Console [VJ Mode]", 0x180 square, only when `vj_mode` is 1 (default 0). Not created by default. | binary [N] |
| Desktop-mode helpers | A keyboard-input window titled "DESKTOP MODE KEYBOARD INPUT WINDOW" and a shell hook, in desktop mode only. In one path of the plug-in (reached through a window it keeps for desktop mode, RVA 0x2B6E4-0x2B77C) it temporarily **raises the priority class of the process that owns that window to the highest class, pumps messages for about 50 ms and restores it**; not on the default path (reading). | binary [A for the code, N for when it runs] |
| Dialogs | Every failure shows a **blocking `MessageBoxW`** (type: foreground + topmost; owner = the plug-in window or none; the caption is string 14, "MILKDROP ERROR"). The string-602 box is a **yes/no question** and "Yes" tries to open a web page (section 2.5 C3). The host must keep dismissing them (choosing the negative button for 602) and treat the following `init` result as failure. | binary [A] |
| Message pump | The window procedure answers input; the per-frame `render` path does not pump messages (the only message pumping code is the desktop-mode path above). The host's `init` thread must **dispatch window messages between `render` calls** (S09 5.3 step 4a), otherwise the plug-in window is unresponsive and the right-click menu and keys do nothing. | binary [A] for "render does not pump"; reading for the effect |
| Quit message | The window procedure, on the destroy message **not caused by the plug-in itself**, calls `PostQuitMessage(0)` and marks the window as closed. Closing the window with its close box or the menu's Quit therefore puts a quit message in the queue of the thread that created it. The host's loop must treat that as "the plug-in wants to stop": call `quit`, then unload (S09 5.3 step 4a, 5). | binary [A] |
| Scroll Lock | Once per frame in which the per-frame routine runs, the plug-in compares the machine's Scroll Lock state with its own state and, when different, sends **synthetic key events** (`keybd_event`, virtual key 0x91 down and up). The default expectation is "off" (setting `bPresetLockOnAtStartup` 0). At shutdown it restores the state it found. INI key `m_bPreventScollLockHandling` = 1 disables this. | binary [A for the routine, its three callers and the disabling key; N for the default of the other key] |
| Foreground | At `quit` the plug-in calls `SetFocus`, `SetActiveWindow`, `SetForegroundWindow` on the main window. | binary [A] |
| Floating-point mode | The device is created **without** the flag value 0x2 ("preserve FPU state"): the behaviour word is 0x20 or 0x80. So the x87 precision of the creating thread drops to single. A host that does x87 double arithmetic on that thread (a 32-bit build compiled for x87) is affected; one that uses SSE is not. | binary [A for the flag word; reading for the effect, an operating-system rule] |

## 8. DirectX facts

### 8.1 Device

| Item | Fact | Evidence |
|---|---|---|
| Library | `d3d9.dll` by `GetModuleHandleA` then `LoadLibraryA`; `Direct3DCreate9` through a delay-load import with the interface version number 32. No other DirectX library is loaded directly (d3dx9: section 4.3). | binary [A] |
| Adapter | Enumerates adapters, chooses the one whose GUID and device name match the INI (else adapter 0 if the INI is empty). | binary [N] |
| Device type | **Device type 1 only** (the hardware-accelerated type): both the capability query (RVA 0x2CD72) and `CreateDevice` (RVA 0x2D84D) pass the value 1. A machine without such a device gets string 432 ("no valid 3D-accelerated display adapter") and `init` returns 1. No reference or software-rasteriser device type is ever tried. | binary [A] |
| Behaviour flags | The behaviour word is **0x20 (software vertex processing)**, or **0x80 (mixed)** when the capability word reports hardware transform-and-light (bit 0x10000 of the device-capability member). No pure device, no multithreaded flag, no FPU-preserve flag. | binary [A for the values and the condition; reading for the names, which come from the public API documentation] |
| Present parameters | Windowed (modes 1-3), back-buffer size = window client size, the format of the **adapter's display mode** (so any desktop format that passes the device-type check), multisample type from the INI (default none), a depth/stencil buffer only when a format was found for it; the other fields come from settings. | binary [N] |
| Format selection | If the display-mode format fails the device-type check, tries 32-bit ARGB, 32-bit XRGB, 24-bit RGB, 16-bit 565, 555, 1555, 4444 and X4444 in that order; none works = string 431. | binary [N] |
| Measured | The VM trace shows the plug-in **passed** the capability query (IPC 611 comes after it) and, because the later step "fonts and menu" is reached only when context creation returned success, also **created the device**. So the Hyper-V guest, which has no GPU, already offers a device of type 1 to Direct3D 9 for this plug-in. | measured (by trace order) + binary [A] |

### 8.2 Capabilities that decide the path

| Item | Fact | Evidence |
|---|---|---|
| Pixel-shader level | Taken from the low 16 bits of the device's pixel-shader version member (major in the high byte): below 2.0 -> level 0 (no pixel shaders); exactly 2.0 -> "2"; above 2.0 and below 3.0 (2_a / 2_b) -> "3"; 3.0 or higher -> "4". INI `MaxPSVersion` (-1 = automatic) replaces the automatic value. Then two adapter-description lists apply [C]: **list 1** (descriptions containing "GeForce 4", "GeForce FX 52" to "56", "SiS 300/305/630/540/730", "Radeon 8", "Radeon 90" to "94", "Radeon 9550", "Radeon X300") sets a canvas-size divisor (the default 100 becomes 200, which halves the canvas dimensions; reading for the purpose); **list 2**, applied only when `MaxPSVersion` is automatic ("GeForce2", "GeForce3", "GeForce4", "Radeon 7", "Radeon 8", "SiS 661FX_760_741", and "Intel" together with "945G", "915G", "910G", "8291", "8281", "8283", "8284", "8285", "8286"), **forces level 0**. A software adapter whose description matches none of these is not affected. | binary [A] |
| Level 0 | The plug-in skips the shader programs and runs the fixed-function pipeline ("MilkDrop 1" mode): no shader compile, no blur textures, **no vertex declarations** [C]. Presets with shader code lose that part. | binary [A for the branch, reading for the look] |
| Level 2 and up | Compile test and compile of the built-in shader set: vertex profile `vs_1_1`; pixel profiles `ps_2_0`, `ps_2_a`, `ps_3_0` by level (an invalid level maps to `ps_4_0` and fails). Uses `D3DXCompileShader` from the d3dx9 library of section 4.3. A failure shows "Failed to compile pixel shaders using ..." and `init` returns 1. | binary [A for the profile strings and the level split; N for the rest] |
| Vertex declarations | Three vertex declarations are created, **only at levels above 0** [C]; a failure shows string 467, 468 or 469 and `init` returns 1. | binary [A] |
| Texture formats | The canvas pair is created as **render-target textures** (usage render target, default pool, one mip level) in a format number chosen by the INI key `nTexBitsPerCh`: 5 -> 23, **8 (default) -> 22**, 10 -> 35, 16 -> 113, 32 -> 116, anything else -> 22 [C: the draft said "the back-buffer format"]. If that creation fails the plug-in retries with the format of the back buffer, then halves the size down to 16 pixels. Level 2+ adds six blur textures, a noise texture and a 3-D noise texture (strings 487-493) [N]. | binary [A for the canvas pair; N for the rest] |
| Texture size | The canvas size is derived from the window size and the device's maximum texture width and height; if the first size cannot be created it halves down to 16 pixels; failure = "not enough video memory" message. | binary [A] |
| Direct3D fonts | Six `D3DXCreateFontW` objects (Courier, Times New Roman, MS Sans Serif, Arial); one failure = string 452 and `init` returns 1. GDI font creation (`CreateFontW`) also must work (string 444). | binary [A for 444, N for 452] |
| Vendor query | The adapter description string is read for the lists above; nothing else of the adapter identity matters. | binary [A] |

### 8.3 Software-renderer verdict

The binary imposes no condition that a software-class device of type 1 cannot
meet (whether the Windows basic render driver is such a device is an operating-
system fact the binary does not show): device type 1, software vertex
processing when no hardware transform is reported, any pixel-shader level
including none, render-target textures in a common 32-bit format, d3dx9 24-36
or a host-supplied module. What the VM has **not** yet shown: the guest's
pixel-shader version, whether the shader compiles succeed in the d3dx9 library
used, whether `CreateTexture` of the canvas succeeds and whether `Present`
puts a picture on the guest's display. They are the next measurement (section
12).

## 9. The render step and the data arrays

| Item | Fact | Evidence |
|---|---|---|
| Module values | Spectrum rows 0, waveform rows 2, latency 0 (rewritten at run time, section 3 step 0), delay 10 ms. The host therefore fills **only the two waveform rows**; the spectrum rows may stay untouched. | binary [A] (static data of the module record) |
| Call | `render` (RVA 0x34C2D) passes the addresses of waveform row 0 and row 1 (module offsets 0x4A4 and 0x6E4, 576 bytes apart) to the render routine (RVA 0x30842). **No other member of the record is read after `init`**: the only references to the record in the whole code are the window-handle member, the instance member (both read at `init`) and the latency member (written). It returns **0 to continue, 1 when that routine reports failure**: failure = the plug-in object is not initialised (for example the window was destroyed) or the device could not be reset or recovered. A 1 makes the host call `quit` (S09 5.3). | binary [A] |
| Skipped frames | When the device reports "lost" or "not reset" the routine waits 30 ms and returns success without drawing; it resets and re-allocates when the device allows it. | binary [A for the 30 ms wait path, N for the reset detail] |
| Interpretation | Each of the 2 x 576 bytes is read as a **signed 8-bit** sample (0x80 = -128, 0 = silence, 0xFF = -1) and converted to a float; both rows are used. The plug-in runs its own transforms on the waveform data (a transform routine at RVA 0x2E166 is called on both rows, and the per-frame routine at RVA 0x1F0CC transforms a copy again), sums three bands of the transform (the loop bounds give bins 0-84, 85-169, 170-255: bass, mid, treb) and keeps an immediate, a short-term and a long-term average per band. The numbers the presets see as `bass`, `mid`, `treb` (and the `_att` forms) are the immediate value divided by the long-term average. Which row feeds the per-frame transform, and whether the render routine's pairwise averaging reaches it, was not settled. | binary [A for the sign handling and the 576 x 2 layout; N for band edges and row use] |
| All zero | No special case, no rejection. Transform and bands are zero. The relative values have a **guard**: when the long-term average is smaller than **0.001** in magnitude the relative value is set to **1.0** (the same guard for the attenuated form). So all-zero input gives constant relative levels of 1.0 ("normal loudness"), no division by zero and no NaN. The waveform is drawn as a flat line; motion and colours are time-driven. | binary [A for the guard and its constant] |
| Constant, non-zero | Reading: a constant offset puts all its energy into transform bin 0, which lies in the **bass** band, so the first frames show a bass level well above the average, and the relative value falls back towards 1.0 as the long-term average follows. No fault, no stall is expected. | reading |
| Waveform alignment | A routine (RVA 0x33130) compares each frame with the previous one to find the best shift; for zero or constant data every shift scores alike and no special case is taken. | binary [N] |
| Other host data needed per frame | The current title (section 4.6). Nothing else: no spectrum, no time, no sample rate (the record's rate and channel members are not read; see the Call row). | binary [A] |

## 10. Cadence, threads and quit

| Item | Fact | Evidence |
|---|---|---|
| Frame limiter | After drawing, the plug-in waits to reach at most **30 frames per second** (INI `max_fps_w`; default 30): a performance-counter spin with 5 ms, 1 ms and 0 ms sleeps, or a single sleep when no counter exists. The host's own `delay_ms` (10) adds to this. The first `render` calls can take much longer (first preset, shader compile). | binary [N] |
| Timer resolution | `timeBeginPeriod(1)` at init, `timeEndPeriod(1)` at quit. Both are process-wide. | binary [A] |
| Worker thread | One thread (`_beginthreadex`, RVA 0x1D6B5) that scans the preset folder and reads each preset's header for its rating; started in step 10, at raised priority. `init` waits until the scan has seen **30 presets or finished**, then continues while the thread works on at a lower priority. It ends by itself; on `quit` (and when the folder changes) the plug-in sets a stop flag, waits up to the given time (0 at quit) and then **terminates it**. The thread and the per-frame routine share one critical section. | binary [A] |
| Timers, hooks | None of these: no `SetTimer`, no timer queue, no hook-installing import; the only other thread-related imports are those of the worker thread. The desktop-mode hook lives in the delay-loaded `vms_desktop.dll`. | binary [A] (import table) |
| `quit` (RVA 0x34C49, then 0x2FF0D) | Releases textures, vertex data and the device; sends **IPC 611 with wparam 0** and destroys the plug-in window **on the calling thread** (so `quit` must be called on the thread that called `init`; the destroy call itself is [A], the thread rule is an operating-system rule, reading); `timeEndPeriod`; stops the scan thread; deletes the GDI fonts; destroys the menu handle of section 2; frees the plug-in's context; restores Scroll Lock; sets the foreground to the main window; sets the "finished" flag that the next `init` waits for. A `quit` without a window left (already destroyed by the user) is safe. | binary [A] |
| Re-start | A new `init` right after `quit` is allowed; the one-second wait of step 3 covers a `quit` still running. | binary [A] |
| What runs for minutes | Nothing in the plug-in itself: it renders until `render` returns 1, the quit message arrives, or the host stops it. Presets change by themselves after the built-in minimum of 16 s plus a random extra (blend 2.7 s; the built-in defaults, overridable by the INI); each change reads one preset file and, at level 2+, compiles two shaders, which can take a noticeable moment on a software device. | reading for the interval; binary [N] |

## 11. What can still stop start-up (cause -> shown text)

| Cause | Where | Shown | Evidence |
|---|---|---|---|
| menu not loaded / no first sub-menu | step 8 | strings 445 / 446 | binary [A]; measured (445) |
| `include.fx` missing | step 9 | string 466, the path | binary [A] |
| any other `.fx` missing | step 9 | string 466 (dialog only), then possibly the compile failure below | binary [A] |
| no device of type 1 / caps query fails | step 7 | string 432 | binary [A] |
| device creation fails | step 7 | strings 438-441 | binary [N] |
| no d3dx9 library found by name (24-36) and IPC 3044 answered 0 or 1 | step 7 | string 602 (yes/no) | binary [A] |
| version below 0x4000 and nothing playing | step 4 | strings 464/465 | binary [A] |
| `D3DXCreateFontW` fails | step 10 | string 452 | binary [N] |
| vertex declaration fails (level above 0) | step 10 | strings 467-469 | binary [A] |
| shader compile fails (level 2+) | step 10 | string 474 and the per-shader strings | binary [N] |
| canvas texture cannot be created | step 10 | strings 483-485 | binary [N] |
| window class registration fails | step 6 | string 430 | binary [A] |

## 12. Open questions

| # | Question | What would settle it |
|---|---|---|
| 1 | Does the guest's Direct3D 9 report a pixel-shader level, and which? Do the shaders compile in the d3dx9 library the guest loads? | A VM run after the host answers message 52 and the data folder is staged: the trace will show the next failure or the first frames; a screenshot shows whether anything is drawn. |
| 2 | Does `Present` put a picture on the guest's display (software device, hidden or covered window)? | Same run, the screenshots of the existing test. |
| 3 | The staged test folder holds only the DLLs: the eight `Milkdrop2\data\*.fx` files (section 6.1, 12 KB in all) must be extracted from the 5.66 installer into `Milkdrop2\data` next to the DLL (or answered through IPC 1336). Presets and textures are optional for a first frame. | The orchestrator's staging step. |
| 4 | Which d3dx9 library loads in the guest (24-36 by name from the plug-in's folder or the system, or the host's own module through 3044), and whether the 43 build works as the answer to 3044 together with its compiler library. | A run with each; the load trace of the process. |
| 5 | The exact code that turns the band sums into the per-frame preset variables was found only at its guard (section 9); the band edges and the transient for constant input are readings. | A VM run with constant input frames. |
| 6 | The behaviour of the length-limited copy with a 0 or 1 answer for IPC 213. | A run with the host answering 1. |
| 7 | What message 30 of the 57B7A1B6 service does (tag, 10, URL, flag byte). | Only matters if URL actions are wanted. |
| 8 | Whether a 5.x-style answer for IPC 0 (>= 0x5051) is wanted in the plug-in host (it removes step 4's dependency on the player state). | A decision, not a measurement. |
| 9 | What the plug-in's per-frame rewrite of the latency member (500, 8 or a computed number) is meant to achieve and whether the host's frame selection follows it. | A run with frames carrying distinguishable timestamps. |
| 10 | Whether a missing `.fx` file other than `include.fx` really ends in a compile failure at the pixel-shader level the guest gets. | A run with one such file removed. |
