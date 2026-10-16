---
Spec ID: S-10
Title: The classic IPC surface (WM_WA_IPC and the plugin-relevant window messages)
Sources consulted (category only): dirty room: public plugin-header tag index and S10 seed note (names, numbers, min versions); dirty room: binary behaviour capture (sandbox runs 20261016-022004, 20261016-023349, 20261016-060342, 20261016-065637, 20261016-070535, 20261016-070243, 20261016-070703); dirty room: offline pixel recomputation on the runs' own captures; dirty room: interpretation audits S10-ipc-replies.md and S10-batches.md; dirty room: prior audited specs S01 (round-2 supplement) and S02; clean repo: audited S09 (plugin ABI), RFC 0009, crates rustamp-plugin-abi / rustamp-ipc / rustamp-plughost, HANDOFF
Author agent: dirty-room spec agent (S10x lane; revision of the measured S10 draft)
Auditor: pending independent ratification (lane audit pass: audits/S10-ipc-audit.md, this lane)
Winamp version validated against: 2.95 (sandbox reference, install C:\winamp, read-only mapping C:\winamp295; the same reference copy S06 and S07 identify as winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561; landmark gate region 0,49,275,67 passed with 0 differing pixels in every run below)
Status: revised draft — measured baseline with the lane-audit corrections (E1–E9) applied; pending Spec Auditor ratification; not final while the open questions of §13 stand, including the version-constant in-process cross-check
---

# S10 — The classic IPC surface

Scope: which window messages the classic plugin talks to its host with — the
`WM_USER`-range IPC channel, the end-of-track notification, the `WM_COPYDATA`
file-enqueue forms and the `WM_COMMAND` mechanism — what each measured id
expects, returns and means in a 2.95 reference, and which ids a plugin host
must answer, which the player answers, and which need an in-process caller.
The plugin ABI itself (structs, entry points, lifecycles) is S09; the
`WM_COMMAND` command-id list is S04; embedding is S11.

This spec states every load-bearing reply with its step, run and reference
state. A value read from a mismatched assertion is a measurement like any
other: the step asserted a hypothesis, the reference corrected it, and the
actual is the fact (marked **by-design mismatch**). A pointer or handle reply
is a **shape-only** reading until an in-process caller reads its contents — a
cross-process probe cannot do that, and this spec does not pretend otherwise.
A **gap is never promoted to a fact**: where the draft that this revision
replaces asserted more than the runs measured (a set reply, an id-502
meaning, a pan input range), the corrected statement is the one below.

---

## 0. How to read this spec

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct measurement in a named run and step, reproduced by a second run where one exists; the reference state is stated |
| **provisional** | Measured but state-dependent (the state is stated) or environment-dependent; the missing piece is named |
| **open** | No 2.95 runtime evidence of any kind; the capture that would settle it is named; no negative result is implied |
| **shape-only** | A pointer/handle reply whose value was read but whose contents are out of reach cross-process |
| **corroboration** | A prior audited spec's measurement (S02, S01 round-2 supplement) that this spec's runs did not re-measure |
| **by-design mismatch** | The job asserted a hypothesis and the reference corrected it; the actual is the measurement |

Provenance tags:

| Tag | Meaning |
|---|---|
| **[IDX]** | From the dirty-room tag index derived from the public plugin development headers (names, numbers, min versions). A documented contract, not an observation of 2.95. |
| **[SEED]** | From `notes/S10-seed.md`, which additionally guesses which bundled plugins use an id (a source-level guess, not evidence). |
| **[S01d]** | Recorded by the S01 round-2 supplement: the playlist-length query (id 124) returned 8 for an 8-entry list. |
| **[S02]** | Recorded by audited S02: `IPC_ISPLAYING` (104) read back 1/3/0 in the three transport states, and `IPC_GETLISTPOS` (125) moves by one on next/prev. |
| **[S09]** | From audited `specs/S09-plugin-abi.md` (current text). |
| **[REPO]** | From the clean repo (`crates/rustamp-plugin-abi`, `crates/rustamp-ipc`, `crates/rustamp-plughost`). |
| **[JOB]** | The id name comes from the capture job, not the seed (ids 502, 900, 999). |
| **[R1]** | Run `20261016-022004` (job `s10-ipc.json` rev 1). 68/69 steps; the single failure is the version probe, whose actual (10501 vs expected 8341) is the run's first version measurement. No reply was read by any other step (rev 1 declared no `expect_reply`). |
| **[R2]** | Run `20261016-023349` (rev 2). 40 steps ok, 29 by-design reply mismatches (exit 4), 0 skipped; **58 measured replies** (29 assertion matches in the manifest + 29 mismatch actuals in the steps' stderr). |
| **[R3]** | Run `20261016-060342` (rev 3, the confirmation run). 63/69 ok, 6 by-design mismatches (the per-boot handle/pointer probes), 0 skipped; **58 measured replies** (52 manifest + 6 stderr). |
| **[B1a]** | Run `20261016-065637` (EQ-window batch, first attempt). The `eq_select_band0` step sent id 127 to the `Winamp EQ` window and got reply 0 — the no-forwarding finding (§5.1). |
| **[B1]** | Run `20261016-070535` (revised EQ-window batch, 6/6 ok, exit 0). The stateful EQ write with its drawn proof. |
| **[B2]** | Run `20261016-070243` (posted-vs-sent, job `s10b-posted-vs-sent.json`). 16 steps, 15 clean, 1 (`eof_post`) failed its `expect_change` gate by design (0 px is the measurement), run exit 5. |
| **[B3]** | Run `20261016-070703` (destructive, job `s10b-destructive.json`). 9/10 steps ok; `del_action`/`pos_action` failed `expect_change` by design (0 px); `restart_action` exit 3 mid-restart; run exit 5. |

Reference states used by the runs, named throughout: **fresh** (fresh launch,
no playlist, stopped, EQ flat with enabled and autoload on, volume 0, pan 0,
shuffle/repeat off) and **loaded** (the 60-entry `r_o4_60.m3u` loaded, settled
post-list: stopped on the last entry, repeat off).

### 0.1 Method note: the harness `send-message` primitive, and why mismatches are measurements

The harness `send-message` step sends **synchronously** (`SendMessageTimeoutW`)
only when the step declares `expect_reply`; without it the step merely
**posts** (`PostMessageW`) and records `reply` as the empty string — no reply
is read at all. There is no read-only mode, so an unknown reply can only be
read by asserting a hypothesis and taking the actual from the step's stderr on
mismatch. Mismatches are recorded as step failures (exit 4) by design; a
mismatch is the measurement, and this spec cites such values as **by-design
mismatch** [E-source]. Two harness facts the next capture batch must obey:

- the manifest's `reply` is a **zero-extended unsigned 32-bit decimal string**
  (0..4294967295): the reference is a 32-bit process and the sender is 64-bit,
  so a 32-bit reply of −1 arrives as **4294967295**, and a step that asserts a
  negative number can never match. Assert the unsigned form (§5.5) [R2 E; R3 M].
- the id goes in the **second** parameter (`lparam`), the argument in the
  **first** (`wparam`); the job fields are signed 32-bit. Getting the order
  backwards is the single most likely silent error in this area [IDX][SEED].

### 0.2 Corrections carried from the lane audit

The revision this spec replaces (the measured S10 draft) is corrected where the
runs say otherwise; the corrections are the lane audit's E1–E9
(`audits/S10-ipc-audit.md`, this lane):

- **E1** — ten steps the draft tagged `[R2 M]` were R2 by-design mismatches
  (`unknown_1/9999/neg/max`, 900, 999, 502, 127-enabled, 127-autoload,
  pan-read-back-after-255). This spec tags them `[R2 E; R3 M]`; the values are
  unchanged.
- **E2** — B2 is 16 steps with 15 clean and the `eof_post` `expect_change`
  failure by design (run exit 5), not "16/16 ok, exit 0".
- **E3** — the full set of R2's 29 by-design corrections is stated in §3.7b,
  not a six-row subset.
- **E4** — the setter steps were posted by design, so **the set calls' own
  replies were never read**. Settled is the setter→read-back contract (§6);
  what a *sent* 122/123 returns (old or new value) is open (§13.11).
- **E5** — id 502's reply is 1; the draft's "(the 'unsupported' convention)"
  gloss is an interpretation in tension with 260 returning a real video window
  handle in the same runs, so the meaning is open (§13.7).
- **E6** — the repo state in §11 is the current one (RFC 0009 applied; the
  plughost answering surface exists), not the pre-RFC description.
- **E7** — S09 §8.3 (current text) already states 0x2905 and marks 0x2095
  superseded; the version item is therefore a measured-and-aligned fact with an
  open cross-check, not a live contradiction with S09.
- **E8** — the 2026-10-15 method memo named by the task card does not exist;
  the 2026-10-16 reconstruction note that does exist covers the `WM_COMMAND`
  ids (O5–O15), is explicitly RFC/handoff-derived, and supplies no evidence for
  the `WM_USER` ids.
- **E9** — pan is settled as **signed at the 255 edge** by measurement (inputs
  0, 64, 255 and the −666 query were sent); the full accepted input range is
  the [IDX] contract (−127..127), not independently measured.

---

## 1. The channel

### 1.1 Message number and parameter order

An "IPC call" is a message with number **`WM_USER` (0x0400)**, public name
`WM_WA_IPC` [IDX][S09], sent to the main window (class `Winamp v1.x` [S09]).
The convention is the reverse of the usual Windows one: the **IPC number**
(id) goes in the **second** parameter (`LPARAM`), the **argument** (if any) in
the **first** (`WPARAM`); both are 32-bit in the 2.x process [IDX][SEED].

### 1.2 Send or post, and how a reply is read

A query's answer is the window procedure's return value, available to the
sender only when the message is **sent synchronously**. `PostMessage` returns
only "queued" and cannot carry a reply back. Therefore a **query** must be
sent to be readable; a **command** may be posted or sent, and its effect is
the observable state. Which of the two 2.95 *requires* per id is partly
measured: for the query ids tested in B2, posting has the same state effect as
sending (§5.2), and the posted setters took effect (§6). For the untested
command ids the question stays open (§13.12).

The end-of-track notification is **not** the IPC channel: it is message number
**`0x0402`** (`WM_USER+2`, public name `WM_WA_MPEG_EOF`), posted by the input
plugin's decode thread with both parameters zero; the host must handle it as a
separate message [S09 §3.6][SEED]. Whether posting it advances the playlist is
measured for one state only (§7.6).

### 1.3 The EQ window does not forward `WM_USER`

**Settled (single-timing measurement) [B1a]:** sending `WM_USER` id 127 to the
`Winamp EQ` window returns **0** (the `DefWindowProc` default for an unhandled
message), not the 31 the main window returns; the step's stdout records the
target class `Winamp EQ`. The EQ window is a capture target only; **IPC
messages must be sent to the main window** (`Winamp v1.x`). The revised batch
sent all IPC to the main window and captured the EQ window separately [B1].

### 1.4 Cross-process limit: pointer arguments are shape-only

Several ids take or return a pointer into the caller's address space (a
`MAX_PATH` buffer to fill, a struct to read, a path pointer to return). A
pointer valid inside a loaded plugin is meaningless to an external process, so
**no out-of-process probe can validate the contents** of the pointer-argument
ids (§8). Such ids can at most be observed to return a non-null pointer of the
expected shape, and this spec marks those readings **shape-only**. Confirming
contents needs an in-process caller — a plugin DLL loaded by the host — which
the plugin host now exists for (§8.3, §11).

### 1.5 Unknown ids, out-of-range ids, negative arguments

**Settled [R2 E; R3 M]:** an undefined number in the classic range (1, 9999),
a number outside the range entirely (65536 probed as a 5.x id, 0x7FFFFFFF) and
a negative IPC number (0xFFFFFFFF) all answer **1**; the 5.x-era probes 801,
3015, 3025 and 65536 likewise answer 1. This **refutes the S09 §8.4/U-6 guess
of "probably 0"** [S09] and matches the 5.x "1 = unsupported" convention
[S09][SEED]. A 2.95-level host that returned 0 where the reference returns 1
changes what a plugin's fallback path does, so this is contract-affecting.

A **negative `WPARAM`** (the −666 query form of 122/123) works: both answer 0
for a fresh reference [R2 M; R3 M]. The argument is sent sign-extended as a
signed 32-bit value; the reply is read as an unsigned 32-bit value (§0.1).

---

## 2. The answerer model

In our architecture the plugin DLL runs in a 32-bit helper (`rustamp-plughost`)
that emulates the Winamp main window, while the real player state lives in the
64-bit engine. Every IPC id is assigned to one of:

| Class | Meaning | Consequence |
|---|---|---|
| **answered-by-player** | Engine state (playing, playlist, time, volume, EQ, shuffle/repeat, stream info). The host forwards the call to the engine and returns the engine's answer. | Needs the host↔engine channel; today the placeholder `Message::WinampIpc` [REPO]. |
| **emulated-by-host** | A property of the emulated window or host process (version constant, ini path, window/menu handles, plugin registration). | The host answers locally; the engine must never see it. |
| **host→plugin notification** | The host calls *out* to a loaded plugin (subclass/timer path); not a request the plugin makes. | Belongs to the host, not the engine. |
| **unknown** | Not yet decided. | Open question; do not implement from this spec. |

The per-id assignment below is a **proposal for ratification**, not a measured
fact: two groups are architectural and stay open — **window handles** (260,
261) and **path/handle ids** (281, 334, 335, 505) — because what is measured is
what the *reference* exposes, and how *our* host supplies handles valid in the
plugin's process is an M3 decision (§13.8).

The placeholder in the repo is explicit that this is S10's job:
`Message::WinampIpc { msg_id, wparam, lparam }` carries a message id and two
zero-extended 64-bit arguments, and its doc comment says the semantics "are
defined by spec S10"; `Message::CommandToHost` overlaps it and RFC 0005 §5.4
leaves the relationship open as **O13** [REPO]. S10 must resolve which ids
travel as `WinampIpc` and which are absorbed locally by the host (§11).

---

## 3. The measured replies, per id

One row per id that carries a measured reply. "Step(s)" names the job step(s);
the run tag and source letter identify where the value was read — **M** = the
assertion matched (value in the manifest), **E** = a by-design mismatch whose
actual is in that step's stderr. "Reference state" is **fresh** or **loaded**
(§0). A wrapped negative is shown as the manifest value with its 32-bit
meaning in parentheses.

| Id | Name | Measured reply | Step(s) (run, src) | Reference state | Status |
|---|---|---|---|---|---|
| 0 | `IPC_GETVERSION` | 10501 (`0x2905`) | `s10_000_getversion` (R1 E; R2 M; R3 M) | fresh | **settled** as measured (three runs agree); the in-process cross-check is open (§5.8, §13.1) |
| 1 | *(undefined probe)* | 1 | `s10_unknown_1` (R2 E; R3 M) | fresh | **settled** — undefined id answers 1 (§1.5) |
| 101 | `IPC_DELETE` | no reply (posted); main window 0 px | `del_action` (B3) | loaded | **open** — job-design artefact: the playlist is the `Winamp PE` window, which was not captured (§7.4, §13.5) |
| 102 | `IPC_STARTPLAY` | no reply (posted); main window +359 px | `play_action` (B3) | loaded | **settled** that it changes the main window; the reply was not read (posted) (§7.2, §13.2) |
| 104 | `IPC_ISPLAYING` | 0 (wparam 0 and −1) | `s10_104_isplaying`, `s10_104_isplaying_wparammax` (R2 M; R3 M); `q104_sent` (B2 M) | fresh; loaded | **settled** for stopped = 0 with the argument ignored; 1 = playing, 3 = paused are corroboration [S02] |
| 105 | `IPC_GETOUTPUTTIME` | mode 0 `4294967295` (−1); mode 1 `4294967295` (−1); mode 2 `0` | `s10_105_time_mode0/1/2` (R2 E; R3 M) | fresh | **provisional** — stopped, no-playlist state; a playing reference is needed (§13.2) |
| 106 | `IPC_JUMPTOTIME` | `4294967295` (−1) | `seek_action` (B3 M) | loaded | **provisional** — stopped; the other returns (1 = end of file, 0 = ok) need a playing reference (§13.2) |
| 109 | `IPC_GETMODULENAME` | 0 | `s10_109_getmodulename` (R2 M; R3 M) | fresh | **provisional** — unsupported *or* no module loaded; not separable without a loaded/playing reference (§13.2) |
| 121 | `IPC_SETPLAYLISTPOS` | no reply (posted); main window 0 px | `pos_action` (B3) | loaded | **open** — ambiguous: a real "no visible effect while stopped" measurement or a PE-window artefact; the main-window capture cannot tell (§7.5, §13.5) |
| 122 | `IPC_SETVOLUME` | query −666 → 0; read-backs 64 / 0 / 255 | `s10_122_volume_query`, `s10_readvol_after64/0/255` (R2 M; R3 M) | fresh | **settled** — unsigned 0..255, echoing read-back (§5.6); the sent set reply itself is unmeasured (§13.11) |
| 123 | `IPC_SETPANNING` | query −666 → 0; read-backs 0 / 64 / `4294967295` (−1) | `s10_123_pan_query`, `s10_readpan_after0/64/255` (R2: query/0/64 M, after-255 E; R3 M) | fresh | **settled** — signed at the 255 edge (§5.6); the full input range is [IDX], not measured (E9) |
| 124 | `IPC_GETLISTLENGTH` | 0 (fresh); 60 (loaded) | `s10_124_getlistlength` (R2 M; R3 M); `q124_sent` (B2 M) | fresh; loaded | **settled** — both states measured; prior evidence [S01d] (8 for an 8-entry list) |
| 125 | `IPC_GETLISTPOS` | 0 | `s10_125_getlistpos` (R2 M; R3 M) | fresh | **settled** for 0 fresh; the ±1 movement on next/prev is corroboration [S02] |
| 126 | `IPC_GETINFO` | 0 in modes 0/1/2 | `s10_126_info_mode0/1/2` (R2 M; R3 M) | fresh | **provisional** — stopped, no stream; a playing reference is needed (§13.2) |
| 127 | `IPC_GETEQDATA` | band 0 `31`, band 9 `31`, preamp `31`, enabled `1`, autoload `1`, selector 99 `0`; after the 128 write of 0: band 0 `0` | `s10_127_eq_band0/band9/preamp/pos99` (R2 M; R3 M), `s10_127_eq_enabled/autoloa d` (R2 E; R3 M), `s10_eq_select_band0`, `s10_eq_read_band0` (R3 M), `eq_select_band0`, `eq_read_band0` (B1 M) | fresh | **settled** — flat state and stateful read-back (§5.7) |
| 128 | `IPC_SETEQDATA` | no reply (posted); the write of 0 over 31 landed (read-back 0) | `s10_eq_write_band0_0` (R3, posted); `eq_write_band0_0` (B1, posted) | fresh | **settled** — the stateful write is conclusive (§5.7) |
| 135 | `IPC_RESTARTWINAMP` | no reply; the capture failed mid-restart (exit 3, `GetClientRect failed`) | `restart_action` (B3) | loaded | **settled** that it restarts the process — the old window is gone before the replacement appears; the capture failure is the measurement (§7.3) |
| 242 | `IPC_INETAVAILABLE` | 1 | `s10_242_inetavailable` (R2 E; R3 M) | fresh | **provisional** — sandbox-networking-dependent; **not** a fixture property, do not export it as one (§13.7) |
| 250 | `IPC_GET_SHUFFLE` | 0; read-backs 1 / 0 after setting | `s10_250_get_shuffle`, `s10_readshuffle_after1/0` (R2 M; R3 M) | fresh | **settled** |
| 251 | `IPC_GET_REPEAT` | 0; read-backs 1 / 0 after setting | `s10_251_get_repeat`, `s10_readrepeat_after1/0` (R2 M; R3 M) | fresh | **settled** |
| 252 | `IPC_SET_SHUFFLE` | no reply (posted); effect read back via 250 (1 then 0) | `s10_setshuffle_1/0` (R2, posted; R3, posted) | fresh | **settled** — posted delivery works (§6) |
| 253 | `IPC_SET_REPEAT` | no reply (posted); effect read back via 251 (1 then 0) | `s10_setrepeat_1/0` (R2, posted; R3, posted) | fresh | **settled** — posted delivery works (§6) |
| 260 | `IPC_GETWND` | EQ `66262`, playlist `66264`, video `66266` (both runs); minibrowser `66270` (R2) / `66268` (R3) | `s10_260_getwnd_eq/pe/mb/video` (R2 E; R3 E) | fresh | **settled** as real, distinct, per-boot window handles; **shape-only** — ownership in our host is open (§13.8) |
| 261 | `IPC_ISWNDVISIBLE` | 1 for EQ (selector 0), playlist (1), main (4) | `s10_261_iswndvisible_eq/pe/main` (R2 E; R3 M); `q261_sent` (B2 M, selector 4) | fresh; loaded (main) | **settled** as the reply value; whether the EQ/playlist windows are *actually* visible in the fresh state is not established (§13.9) |
| 281 | `IPC_GET_HMENU` | `262789` (R2); `328057` (R3) | `s10_281_get_hmenu` (R2 E; R3 E) | fresh | **settled** as a non-null per-boot menu handle; **shape-only** — ownership is an M3 decision (§13.8) |
| 400 | `IPC_ISFULLSTOP` | 0 | `s10_400_isfullstop` (R2 M; R3 M) | fresh | **settled** |
| 501 | `IPC_IS_PLAYING_VIDEO` | 1 | `s10_501_is_playing_video` (R2 E; R3 M) | fresh | **provisional** — the seed's "old host" value, measured with no video playing; a video-playing reference is needed (§13.2) |
| 502 | `IPC_HAS_VIDEO_SUPPORT` [JOB] | 1 | `s10_502_has_video_support` (R2 E; R3 M) | fresh | **settled** as the reply value; the meaning (unsupported vs support-present — 260 returned a real video window handle in the same runs) is open (§13.7) |
| 666 | `IPC_EX_ISRIGHTEXE` | 1 | `s10_666_isrightexe` (R2 E; R3 M) | fresh | **settled** |
| 800 | `IPC_GETSADATAFUNC` | `4407808` (`0x434200`) | `s10_800_sadatafunc` (R2 E; R3 E); `q800_probe` (B2 M) | fresh; loaded (B2, fresh launch) | **provisional** — the value is stable across three independent launches (§5.3); its meaning (the seed marks 800 as 5.0+) and its contents are unmeasured without an in-process caller (§13.3) |
| 801 | `IPC_GETVUDATAFUNC` | 1 | `s10_801_vudatafunc` (R2 E; R3 M) | fresh | **settled** — unsupported (5.0+ id) |
| 900 | `IPC_ISMAINWNDVISIBLE` [JOB] | 1 | `s10_900_ismainwndvisible` (R2 E; R3 M) | fresh | **settled** as the reply value; the name is job-derived [JOB] |
| 999 | `IPC_IS_SAFEMODE` [JOB] | 1 | `s10_999_is_safemode` (R2 E; R3 M) | fresh | **settled** as the reply value; the name is job-derived [JOB] |
| 3015 | `IPC_GET_RANDFUNC` | 1 | `s10_3015_randfunc` (R2 E; R3 M) | fresh | **settled** — unsupported (5.1+ id); calling the function it would return needs an in-process caller (§13.3) |
| 3025 | `IPC_GET_API_SERVICE` | 1 | `s10_3025_api_service` (R2 E; R3 M) | fresh | **settled** — matches the "1 = unsupported" convention (5.12+ id) |
| 65536 | `IPC_REGISTER_WINAMP_IPCMESSAGE` | 1 | `s10_65536_register` (R2 E; R3 M) | fresh | **settled** — unsupported (5.0+ id) |
| 9999 | *(undefined probe)* | 1 | `s10_unknown_9999` (R2 E; R3 M) | fresh | **settled** — undefined id answers 1 (§1.5) |
| −1 | *(negative id probe)* | 1 | `s10_unknown_neg` (R2 E; R3 M) | fresh | **settled** — negative id answers 1 (§1.5) |
| 2147483647 | *(out-of-range probe)* | 1 | `s10_unknown_max` (R2 E; R3 M) | fresh | **settled** — out-of-range id answers 1 (§1.5) |

The 11 setter steps that posted by design (`s10_setvol_64/0/255`,
`s10_setpan_0/64/255`, `s10_setshuffle_1/0`, `s10_setrepeat_1/0`,
`s10_eq_write_band0_0`) carry no reply by construction; their effects are §6
and §5.7.

### 3.1 The id-0 version constant (measured; cross-check open)

`IPC_GETVERSION` (id 0) returned **`0x2905` (10501)** in three independent
runs: R1 found it by mismatch (stderr: `10501 does not match expected 8341`),
R2 asserted it and it matched, R3 asserted it and it matched again. The seed
derives `0x2095` from the header convention "0x20yx means 2.yx" [SEED]; that
reading was never measured. S09 §8.3 (current text) states `0x2905`, marks the
`0x2095` reading superseded on this same evidence, and RFC 0009 corrected the
crate constant `WINAMP_VERSION_295` to `0x2905` [S09][REPO]. The two values sit
on opposite sides of the `0x2900` threshold plugins test for 2.9+ features, so
this is contract-affecting: a plugin built against `0x2095` would take the
old-host branch on a host that answers the measured value. **What remains open
is the decisive cross-check — an in-process (plugin-DLL) caller** — which the
plugin host now exists for and has not yet performed (§5.8, §11, §13.1).

### 3.2 The full R2 correction set (by-design mismatches)

R2 (rev 2) asserted the pre-measurement hypotheses and the reference corrected
29 steps; the actuals are the measured facts. The full set, grouped:

| Hypothesis asserted by the job | Steps | Measured actual | Correction |
|---|---|---|---|
| version `0x2095` (R1 asserted 8341; R2/R3 asserted the measured 10501) | `s10_000_getversion` (R1 E; R2 M; R3 M) | 10501 (`0x2905`) | the constant is `0x2905`; three runs agree (§3.1) |
| unsupported id → 0 | `s10_unknown_1/9999/neg/max` (R2 E; R3 M) | 1 | 2.95 answers 1, matching 5.x (§1.5) |
| 105 modes 0/1 → −1 (asserted `-1`); mode 2 → −1 | `s10_105_time_mode0/1/2` (R2 E; R3 M) | 4294967295 / 4294967295 / 0 | the unsigned form of −1 for modes 0/1 (a step cannot assert `-1`, §0.1); mode 2 answers 0 while stopped |
| pan read-back after set 255 → 255 | `s10_readpan_after255` (R2 E; R3 M) | 4294967295 (−1) | pan is signed; 255 wraps to −1 (§5.6) |
| EQ write 31→31 is conclusive | `s10_eq_write_band0_31` + read-back (R2 M) | 31 (matched) | **inconclusive** — the written value equalled the existing value; R3 wrote 0 over 31 and read back 0 (§5.7) |
| 260/281/800 → 0 (probed with 0) | `s10_260_getwnd_eq/pe/mb/video`, `s10_281_get_hmenu`, `s10_800_sadatafunc` (R2 E; R3 E; B2 M for 800) | per-boot handles 66262/66264/66270/66266; 262789; 4407808 | these ids return nonzero per-boot values; a 0-probe detects only "nonzero" |
| 127 enabled → 0; autoload → 0 | `s10_127_eq_enabled`, `s10_127_eq_autoload` (R2 E; R3 M) | 1 / 1 | the fresh reference has EQ enabled and autoload on |
| 242 → 0; 666 → 0; 501 → 0; 502 → 0; 900 → 0; 999 → 0; 801 → 0; 3015 → 0; 3025 → 0; 65536 → 0 (probed with 0) | the ten steps (R2 E; R3 M) | 1 in every case | all ten ids answer 1 on the fresh reference (5.x-era ids follow the unsupported convention; 242 is sandbox-networking-dependent; 501 is the "old host" value; 502's meaning is open) |
| 261 → 0 for EQ/playlist/main | `s10_261_iswndvisible_eq/pe/main` (R2 E; R3 M) | 1 / 1 / 1 | all three selectors report visible in the fresh state |

---

## 4. The non-IPC messages (seed section A)

| Message | Number / selector | Direction | Argument | Return | Who answers (proposed) | 2.95 status |
|---|---|---|---|---|---|---|
| End-of-track notification (`WM_WA_MPEG_EOF`) | message `0x0402` = `WM_USER+2` (not the IPC channel) | input plugin → main window, **posted** | both params 0 | none | host→player | **provisional** [B2]: posting it in the stopped post-list state left the window byte-identical (0 px) — a real measurement for that state, but it does **not** settle [S09] U-1 (§7.6, §13.4) |
| File enqueue (`IPC_PLAYFILE` / `IPC_ENQUEUEFILE`) | `WM_COPYDATA`, `dwData` = 100 | external/plugin → main | `lpData` = NUL-terminated ANSI path or playlist; `cbData` includes terminator | "handled" flag | player | *open* — not probed; a job step is named in §13.13 |
| Change directory (`IPC_CHDIR`) | `WM_COPYDATA`, `dwData` = 103 | external → main | `lpData` = directory path | "handled" flag | unknown (player or host) | *open* — not probed (§13.13) |
| Command message | `WM_COMMAND`, command id in the low word of `WPARAM` | any → main | — | none | player | mechanism only; the id list is S04, not S10 (§9) |
| Private `WM_USER+n` other than +0 and +2 | `0x0401`, `0x0403`, … | plugin → main | unknown | unknown | unknown | *open* — no sweep has been run (§13.10) |

## 5. Findings from the runs

Each finding is one subsection. Provenance tags are §0; the interpretations
they rest on are `audits/S10-ipc-replies.md` (R3) and `audits/S10-batches.md`
(B1, B2, B3), both re-checked against the raw run data in the lane audit.

### 5.1 The EQ window does not forward `WM_USER` — it answers `0` itself

**Settled (single-timing) [B1a].** Sending id 127 to the `Winamp EQ` window
returned 0, not the 31 the main window returns (stderr: `reply 0 does not
match expected 31`; the step's stdout records target class `Winamp EQ`).
Interpretation: the EQ window answers `WM_USER` itself with the
`DefWindowProc` default for an unhandled message; it does not forward `WM_USER`
to the main window's IPC handler. Consequence (§1.3): IPC messages are sent to
the main window; the EQ window is a capture target only. The revised batch
[**B1**, 6/6 ok] sent all IPC to the main window and captured the EQ window in
separate no-action steps.

### 5.2 Posted versus sent — posting a query equals sending it

**Settled for the tested query ids [B2].** Five posted/sent pairs with sent
assertions (id 0 → 10501, 104 → 0, 124 → 60, 127 → 31, 261 → 1), all
`expect_same` satisfied, plus three posted-only steps (260, 281, 800 —
handles/pointers, no sent assertion). For each pair the posted step
(`PostMessageW`, no reply) left the main window byte-identical to the sent step
(`SendMessageTimeoutW`, reply asserted). Settled: **posting a query has the
same state effect as sending it** — a query has no drawn effect, so both
captures are the unchanged window, and the sent reply confirms the id still
answers. Limitation, carried as stated: the comparison still cannot
distinguish "delivered, no effect" from "not delivered" for a query; that
distinction is what a message with a visible effect — the `0x0402` post (§7.6)
— would provide.

### 5.3 The id-800 pointer is stable across three launches

**Settled as value stability [R2 E; R3 E; B2 M].** Id 800 answered
`4407808` (`0x434200`) in R2 and R3 (mismatch actuals, the job had probed 0)
and in B2's `q800_probe` (a fresh launch, assertion matched) — three
independent launches, fresh and loaded states. **Open:** the pointer's
*meaning* — the seed marks 800 as 5.0+, so either the seed's minimum version is
wrong or 2.95 answers 800 for another purpose — and its *contents*, both of
which need an in-process caller (§8.3, §13.3).

### 5.4 The unsupported-id answer is `1` — it refutes the `0` guess

**Settled [R2 E; R3 M].** Undefined ids (1, 9999), a negative id (−1 =
0xFFFFFFFF) and an out-of-range id (2147483647) all answered 1; the 5.x probes
801, 3015, 3025, 65536 (and 502, 900, 999) likewise returned 1. This refutes
S09 §8.4/U-6's "probably 0 from default window-procedure handling" and matches
the 5.x "1 = unsupported" convention [S09]. A 2.95 host that returned 0 where
the reference returns 1 changes what a plugin's fallback path does, so this is
not cosmetic. S09 §8.4 and U-6 need an amendment or cross-reference to this
spec's settled row.

### 5.5 The reply's unsigned-32-bit shape

**Settled as a harness usage fact [R2 E; R3 M].** The manifest's `reply` is an
**unsigned 32-bit decimal string** (0..4294967295); the 32-bit `LRESULT` is
zero-extended by the 64-bit sender. Hence a 32-bit −1 is recorded as
`4294967295`, and a step cannot assert a negative reply (the comparison is on
64-bit integers). Cases in these runs where −1 is the real meaning:
`IPC_GETOUTPUTTIME` (105) modes 0 and 1 (not playing) and the
`IPC_SETPANNING` (123) read-back after setting 255 (signed pan). R2 asserted
−1/255 and the failures were the harness's, not the ids'; R3 asserted the
unsigned forms and matched.

### 5.6 Volume unsigned versus pan signed

**Settled [R2 E/M; R3 M].** Setting volume 255 reads back 255; setting pan 255
reads back `4294967295` (−1); volume 64/0 and pan 0/64 echo exactly.
**Volume is unsigned 0..255; pan is signed** — 255 wraps to −1. This also
explains the R1 pixels: pan 0 and pan 255 render identically (0 px offline
recomputation), because as a signed quantity −1 is next to 0. The seed's
"classic 0..255, later −127..127" [SEED] is answered for 2.95 at the measured
edge: **pan is the signed form**. The full accepted input range is not itself
measured (only 0, 64, 255 and the −666 query were sent); it stays [IDX]
contract (E9).

### 5.7 The stateful EQ write, with its drawn proof

**Settled [R3 M; B1 M + offline px].** The 127→128→127 sequence is conclusive.
R2 wrote 31 over 31 (inconclusive — the written value equalled the existing
one). R3 selected band 0 (reply 31), wrote 0 (id 128, posted), and read back 0
— the written value differs from the flat value, so the read-back proves the
write landed. B1 added the drawn proof with all messages to the main window
(§1.3): `eq_select_band0` (31) → `eq_write_band0_0` (posted) → `eq_drawn`
differs from `eq_flat` by **431 px** (offline recomputation: bbox
(79,17)-(100,98); the run's `expect_change` gate is the in-run evidence) →
`eq_read_band0` (0) → `eq_drawn_read` byte-identical to `eq_drawn`
(`expect_same` satisfied: the read-back did not disturb the drawing). Settled:
the stateful write works (127 selects a slot, 128 writes to it), and its drawn
effect is captured.

### 5.8 The version constant — measured, aligned, cross-check open

The measurement is §3.1: `0x2905` in three independent runs, S09 §8.3 (current
text) aligned, the crate constant corrected by RFC 0009 [S09][REPO]. The
decisive cross-check that remains is an **in-process (plugin-DLL) caller**:
loaded into the reference 2.95 for a dirty-room re-measurement of id 0 and the
pointer ids, and loaded into our host for the RFC 0009 verification. The
plugin host's answering surface now exists for exactly this (§11); no plugin
DLL has yet been loaded in either direction, so the cross-check stays open
(§13.1) and this spec does not promote the external measurement beyond
"settled as measured".

## 6. The setter cross-check (replies and pixels)

The setter→read-back pairs have both halves for all five setters, and the EQ
write is conclusive (§5.7). The pixel column is R1, offline-recomputed
identically by the lane audit; the sprite identities are not named (the bounds
are measured; which drawn element each is needs the S01/S02 geometry).

| Setter | Set to | Read-back (query) | Pixel effect (R1) |
|---|---|---|---|
| volume 122 | 64 | `64` [R2][R3] | set 64 vs set 0: 438 px, bounds (107,58)-(170,68) |
| volume 122 | 0 | `0` [R2][R3] | set 0 reproduced the fresh-launch image exactly (0 px) |
| volume 122 | 255 | `255` [R2][R3] | set 0 vs set 255: 456 px, bounds (107,58)-(171,68) |
| pan 123 | 0 | `0` [R2][R3] | set 0 and set 255 pixel-identical (0 px) |
| pan 123 | 64 | `64` [R2][R3] | set 255 vs set 64: 262 px, bounds (178,58)-(214,68) |
| pan 123 | 255 | **`4294967295` (−1)** [R2 E][R3 M] | (see pan 0 row) |
| shuffle 252 | 1 | `1` (via 250) [R2][R3] | on vs off: 6 px, bounds (170,94)-(172,95) |
| shuffle 252 | 0 | `0` (via 250) [R2][R3] | |
| repeat 253 | 1 | `1` (via 251) [R2][R3] | on vs off: 6 px, bounds (216,94)-(218,95) |
| repeat 253 | 0 | `0` (via 251) [R2][R3] | |
| EQ 127/128 | write 0 on band 0 (R3/B1; R2 wrote 31) | `0` (R3/B1 read-back) | EQ window 431 px vs flat (B1, offline recomputation) |

What this settles, and what it does not:

- **Settled [R2][R3]: the setter→read-back contract holds for volume, pan,
  shuffle and repeat.** Set 64/0/255 and read back 64/0/255 (volume), 0/64/−1
  (pan), and set shuffle or repeat and read back 1, then 0.
- **Not settled (E4): the set calls' own replies.** The setter steps were
  posted by design, so a sent 122/123 was never read: whether it returns the
  old or the new value is open (§13.11). The [IDX] contract says "int (old or
  new value)".
- **Settled [R1][R2][R3]: the posted messages were delivered and acted on.**
  The setter effects are visible in the main window (pixel column) and in the
  read-backs, so posting is a working delivery path for 122/123/252/253/128.
- **Not established: the identity of the changed sprites.** The bounds are
  measured; the shuffle/repeat changes are small (6 px) and this spec does not
  claim a whole button lit up (§13.14).

## 7. The destructive and notification probes (B3 and B2)

| Step | Id | Result | Reading |
|---|---|---|---|
| `play_action` | 102 | +359 px vs `play_before` | **measurement** — `IPC_STARTPLAY` changed the main window (transport/position/title moved); the reply was not read (the step posted) |
| `seek_action` | 106 | reply `4294967295` (−1) | **measurement** — `IPC_JUMPTOTIME` in the stopped post-list state returns −1 ("not playing"); the other documented returns need a playing reference |
| `restart_action` | 135 | exit 3 (`GetClientRect failed`) | **measurement** — the capture failed *while the process restarted*; the restart closed the old window before the replacement appeared. `IPC_RESTARTWINAMP` does restart Winamp; this is the predicted "capture failure is the result" case, not a no-op |
| `del_action` | 101 | 0 px vs `del_before` | **job-design artefact** — the main window (`Winamp v1.x`) does not show the playlist; the playlist is the separate `Winamp PE` window, which the job did not capture. The delete's effect is **unmeasured** (§13.5) |
| `pos_action` | 121 | 0 px vs `pos_before` | **ambiguous** — the evidence cannot separate (a) a real measurement (a bare set-position while stopped does not update the main window's title) from (b) a job-design artefact (the set-position's primary visible effect, the current-entry highlight, is in the PE window). Flagged, not guessed (§13.5) |

### 7.1 The `0x0402` verdict

[B2] `eof_before` captured the settled post-list state (stopped on the last of
60 entries, repeat off); `eof_post` posted message 0x0402 with both parameters
zero and the capture was byte-identical (0 px, offline-recomputed). The 0 px is
a **real measurement for the state tested**, and the job's design is not at
fault — the state it set up is the one state in which a no-op is expected
regardless of 0x0402's semantics (no track playing to "end", no next entry to
advance to). It therefore does **not** settle [S09] U-1: it cannot separate
"0x0402 is a no-op" from "0x0402 advances only while playing / only when a
next entry exists". The named follow-up is a **mid-list probe** (§13.4).

## 8. Pointer and handle ids — shape-only, and what needs an in-process caller

### 8.1 Shape-only readings measured so far

| Id | Shape read | State | Contents |
|---|---|---|---|
| 109 | 0 | fresh | open — unsupported vs no module loaded (§13.2) |
| 260 | four real distinct per-boot window handles | fresh | open — the handles are valid in *the reference's* process; ownership in our host is an M3 decision (§13.8) |
| 281 | non-null per-boot menu handle | fresh | open — ownership is an M3 decision (§13.8) |
| 800 | `0x434200`, stable across three launches | fresh; loaded | open — meaning and contents (§13.3) |
| 801 / 3015 / 3025 | 1 (unsupported) | fresh | n/a — the functions would be called by a plugin; unsupported here |

### 8.2 The out-of-process limit

**No external, out-of-process probe can validate the contents of the
pointer-argument ids:** 100 (struct form), 129, 200, 201, 211, 212, 213, 214,
245, 246, 290, 291, 294, 295, 296, 334, 335, 336, 505, 65536 [IDX][SEED]. It can
at most observe that the call returned a non-null pointer of the expected
shape. **334, 335 and 505 were never probed** — not even a shape reading
exists (§13.6).

### 8.3 What needs an in-process plugin caller — the plugin host now exists for it

The ids that need an **in-process caller** (a real plugin DLL loaded by the
host) to be more than a shape check:

- **202** (`IPC_EXECPLUG`) — needs a plugin DLL loaded;
- **290/294/295/296** (extended-file-info, which forward to the input
  plugin's export) and **291** (struct contents);
- **505** (`IPC_GET_EMBEDIF`) — function pointer / frame handle;
- **800/801/3015** — function-pointer getters: the pointer can be observed
  cross-process, the function cannot be called;
- **the contents of every pointer-argument id in §8.2** (100 struct form, 129,
  200, 201, 211–214, 245, 246, 290, 291, 334, 335, 336, 505, 65536);
- **the version-constant cross-check of id 0** (§5.8) — a plugin measuring
  `IPC_GETVERSION` in-process, both against the reference 2.95 (dirty room) and
  against our host (the RFC 0009 pending cross-check).

**The plugin host now exists for exactly this.** `rustamp-plughost` (i686)
implements a table-driven answering surface for the measured ids —
`crates/rustamp-plughost/src/ipc.rs`, one row per id with the run and step the
value rests on, unknown ids falling through to the S10 convention of 1 — and
its end-to-end test already sends thirteen plugin-shaped IPC queries (version,
playing, output time, volume and pan including the −666 query, list length and
position, stream info, shuffle and repeat) to the host's own window until they
match published state [REPO]. What remains is the probe plugin DLL itself and
the runs: loading a probe plugin into the reference 2.95 (dirty room) to
re-measure id 0 in-process and call/read the pointer ids, and loading a plugin
into our host for the RFC 0009 verification. Neither has been performed; the
cross-check stays open (§13.1, §13.3, §13.6).

## 9. `WM_COMMAND`

The classic `WM_COMMAND` command ids (40012 prefs, 40036 EQ, 40045 play, …)
are the subject of **S04**, not S10. S10 fixes only the mechanism: a plugin or
external tool sends `WM_COMMAND` to the main window with the id in the low word
of `WPARAM`, and a general-purpose plugin may instead intercept it by
subclassing. See S04 for the id list and S03 for which key triggers which id.
The 5.x-only vis-command ids (40382..40391) are sent to the window registered
with id 611 [S09 §5.3][SEED] and are not required of a 2.95-level host.

## 10. Host→plugin notifications (seed section B4)

These are messages the *host* sends to a plugin that subclassed the main
window. All are 5.x in the seed; the classic 2.x era used plain subclassing
and watched `WM_COMMAND`, `WM_TIMER`, `WM_USER` and the end-of-track message.
Whether 2.95 sends any of them is **open** — a 2.95 host is not required to.

| Id | Name | Meaning | 2.95 status |
|---|---|---|---|
| 600 / 601 | `IPC_CB_ONSHOWWND` / `IPC_CB_ONHIDEWND` | a window shown or hidden; `WPARAM` = window selector (0 EQ, 1 PE, 2 MB, 3 video, 4 main) | *open* [IDX][SEED] — not probed |
| 603 | `IPC_CB_MISC` | umbrella notification; `WPARAM` selects the sub-event (0 title/start-stop, 1 volume/pan, 2 status, 3 EQ, 4 info, 5 video info, 6 rating, 7 pause, 8 unpause) | *open* [IDX][SEED] — not probed |
| 618 | `IPC_CB_ONTOGGLEAOT` | always-on-top toggled | *open* [IDX][SEED] — not probed |
| 622 | `IPC_CB_PEINFOTEXT` | playlist info text changed | *open* [IDX][SEED] — not probed |
| 623 | `IPC_CB_OUTPUTCHANGED` | output plugin selection changed | *open* [IDX][SEED] — not probed |

## 11. What our side already speaks

[REPO] The clean repo already fixes part of this surface, and this spec agrees
with it rather than restates it:

- `rustamp-plugin-abi` defines `WM_USER = 0x0400`, `WM_WA_IPC = WM_USER`,
  `WM_WA_MPEG_EOF = WM_USER + 2`, `IPC_GETVERSION = 0` with the 2.95 value
  **`WINAMP_VERSION_295 = 0x2905`** — corrected from the SDK-derived 0x2095 by
  **RFC 0009** on the three-run evidence of §3.1 — the 2.9x threshold
  `WINAMP_VERSION_2_9X_THRESHOLD = 0x2900`, and `IPC_SET_VIS_COMMAND_WINDOW =
  611` (marked 5.x).
- `rustamp-ipc` carries `Message::WinampIpc { msg_id, wparam, lparam }` as an
  explicit **placeholder** whose semantics are "defined by spec S10", and
  `Message::CommandToHost` whose overlap with it is RFC 0005 **O13**.
- `rustamp-plughost` implements the table-driven answering surface
  (`src/ipc.rs`, 39 measured ids) built on this spec's §3 table, with the
  conventions of §5.4 (unknown → 1), §5.5 (unsigned replies), §5.6 (volume
  unsigned, pan signed) and §5.7 (stateful EQ) as row semantics, plus unit
  tests and the end-to-end test of §8.3. That e2e surfaced and fixed a
  32-bit **sign-extension bug**: a negative query argument (the −666 form)
  arrived as a large positive on i686, so volume and pan queries could never
  match [REPO, HANDOFF 2026-10-16].

So this spec's concrete outputs for the host are three, and the surface's
existence changes none of them: the **wParam/lParam marshalling rule** for
`WinampIpc` (32-bit values zero-extended to 64-bit on the wire, per the
placeholder's fields), the **host-vs-player split** for each id (§2, to be
ratified), and the **resolution of O13** (which calls travel as `WinampIpc` and
which are absorbed locally by the host).

## 12. Candidate ids not measured by these runs

The rows below carry no 2.95 measurement; every cell is *open* and no negative
result is implied. "Needs" names the settling capture. The [IDX]/[SEED]
argument/return cells are the documented contract, not observations.

### 12.1 Classic set (seed B1 remainder)

| Id | Name [IDX] | Argument | Return | Who answers (proposed) | Needs |
|---|---|---|---|---|---|
| 100 | `IPC_PLAYFILE` / `IPC_ENQUEUEFILE` (struct form) | struct pointer {path, title, extension, length} | 0 | player | in-process caller (§8.2) |
| 120 | `IPC_WRITEPLAYLIST` | 0 | current position int | player | a job step (sent, expect_reply) |
| 129 | `IPC_ADDBOOKMARK` | path pointer | none | player | in-process caller (§8.2) |
| 130 | `IPC_INSTALLPLUGIN` | unknown | unknown | unknown | a job step; likely needs a plugin DLL |
| 200 | `IPC_SETSKIN` | skin name/path pointer | none | player | in-process caller (plugin-only [IDX]) |
| 201 | `IPC_GETSKIN` | `MAX_PATH` buffer pointer | pointer to the skin directory | player | in-process caller (§8.2) |
| 202 | `IPC_EXECPLUG` | `"library.dll[,index]"` pointer | none | player/host | a plugin DLL (§8.3) |
| 211 | `IPC_GETPLAYLISTFILE` | index | path pointer or null | player | in-process caller (§8.2) |
| 212 | `IPC_GETPLAYLISTTITLE` | index | title pointer | player | in-process caller (§8.2) |
| 240 | `IPC_GETHTTPGETTER` | unknown | function pointer | unknown | in-process caller |
| 241 | `IPC_MBOPEN` | URL pointer or 0 | none | player | a job step (pointer form — shape-only at best) |
| 243 | `IPC_UPDTITLE` | 0 | none | player | a job step (observable title state) |
| 245 | `IPC_CHANGECURRENTFILE` | path pointer | none | player | in-process caller (§8.2) |
| 246 | `IPC_GETMBURL` | 4096-byte buffer pointer | none | player | in-process caller (§8.2) |
| 247 | `IPC_REFRESHPLCACHE` | 0 | none | player | a job step |
| 248 | `IPC_MBBLOCK` | 1 block, 0 unblock | none | player | a job step |
| 249 | `IPC_MBOPENREAL` | URL pointer | none | player | a job step |

### 12.2 2.9x additions (seed B2 remainder)

| Id | Name [IDX] | Argument | Return | Who answers (proposed) | Needs |
|---|---|---|---|---|---|
| 199 | `IPC_SETSKINW` | wide skin name pointer | none | player | in-process caller |
| 214 | `IPC_GETPLAYLISTFILEW` | index | wide path pointer | player | in-process caller (§8.2) |
| 259 | `IPC_ENABLEDISABLE_ALL_WINDOWS` | `0xdeadbeef` disables, anything else enables | none | unknown (host or player) | a job step (observable window state) |
| 280 | `IPC_ADJUST_OPTIONSMENUPOS` | signed delta | int | host (emulated menu) | a job step; the menu handle's ownership is open (§13.8) |
| 290 | `IPC_GET_EXTENDED_FILE_INFO` | struct pointer | 1 if the owning plugin supports it | player (forwards to the input plugin) | a plugin DLL (§8.3) |
| 291 | `IPC_GET_BASIC_FILE_INFO` | struct pointer | int | player | in-process caller (§8.2) |
| 294 | `IPC_SET_EXTENDED_FILE_INFO` | struct pointer | — | player | a plugin DLL (§8.3) |
| 295 | `IPC_WRITE_EXTENDED_FILE_INFO` | 0 | — | player | a plugin DLL (§8.3) |
| 296 | `IPC_GET_EXTENDED_FILE_INFO_HOOKABLE` | struct pointer | — | player | a plugin DLL (§8.3) |
| 334 | `IPC_GETINIFILE` | 0 | path pointer into host memory | host or player | in-process caller; **never probed** (§13.6) |
| 335 | `IPC_GETINIDIRECTORY` | 0 | directory pointer | host or player | in-process caller; **never probed** (§13.6) |
| 500 | `IPC_GET_IVIDEOOUTPUT` | 0 | video-output interface pointer | player | in-process caller |
| 505 | `IPC_GET_EMBEDIF` | 0 → function pointer; window-state struct pointer → frame handle | function pointer or window handle | host (embedding is S11) | in-process caller; **never probed** (§13.6) |
| 506–510 | `IPC_CONVERTFILE` family | mixed | mixed | unknown | a job step (2.92+ conversion) |
| 532 / 533 | `IPC_EMBED_ENUM` / `IPC_EMBED_ISVALID` | mixed | mixed | host (S11) | a job step |

### 12.3 5.x ids the target plugins probe (seed B3 remainder)

A 2.95-level host is not required to implement these, but the plugins named in
the seed (MilkDrop 2, in_openmpt, Geist-era gen plugins) probe them and branch
on the reply, so the host must know what it returns. The measured 5.x probes
(801, 3015, 3025, 65536, 502, 900, 999) are in §3; the rest:

| Id | Name [IDX] | Argument | Return | Who answers (proposed) | Needs |
|---|---|---|---|---|---|
| 336 | `IPC_GETPLUGINDIRECTORY` | 0 | plugin-folder pointer | host | in-process caller; shape probe possible (§13.6) |
| 365 | `IPC_GETDIALOGBOXPARENT` | 0 | window handle | host | in-process caller |
| 611 | `IPC_SETVISWND` | window handle (0 clears) | — | host | a job step (vis window state) |
| 612 / 613 | `IPC_GETVISWND` / `IPC_ISVISRUNNING` | 0 | handle / 0–1 | host | a job step |
| 628 | `IPC_CB_VISRANDOM` | packed status | — | host→player | a job step |
| 631 | `IPC_SET_VIS_FS_FLAG` | 1 entering fullscreen, 0 leaving | — | host | a job step |
| 3003 / 13003 | `IPC_PLAYING_FILE` / `_W` | path | — | player→plugin notification | observed via a loaded plugin |
| 3007 | `IPC_ALLOW_PLAYTRACKING` | allow flag | — | player | a job step |
| 3017 | `IPC_METADATA_CHANGED` | field name or null | — | player→plugin notification | observed via a loaded plugin |
| 3032 | `IPC_OPEN_URL` | URL string | — | player | a job step |
| 3033 | `IPC_USE_UXTHEME_FUNC` | selector or window handle | 0 on success | host | a job step |
| 3044 | `IPC_GET_D3DX9` | 0 | module handle (1 if unsupported) | host | a job step |
| 1335 / 1336 | `IPC_GETINIDIRECTORYW` / `IPC_GETPLUGINDIRECTORYW` | 0 | wide path pointer | host | in-process caller |

## 13. Open questions (with the capture that would settle each)

1. **The version-constant in-process cross-check.** `0x2905` is settled as
   measured (three external runs, §3.1) and aligned with S09 §8.3 and the crate
   (RFC 0009); the decisive cross-check that remains is an **in-process
   (plugin-DLL) caller** — a probe plugin loaded into the reference 2.95 to
   re-measure id 0 from inside the process, and a plugin loaded into our host
   for the RFC 0009 verification. The plugin host's answering surface exists
   for this (§8.3, §11); the probe plugin and the runs do not. **The cross-check
   stays open until both directions are measured; this spec does not promote
   the external measurement beyond "settled as measured".**
2. **The state-dependent ids in a playing state.** 105 (position/length while
   playing; mode 2's playing value), 126 (sample rate/bitrate/channels while a
   stream is open), 109 (separating "unsupported" from "no module loaded"), 501
   (separating the "old host" value from real video), plus 106's other returns
   (1 = end of file, 0 = ok). **Settling capture:** a loaded *and playing*
   reference pass of the §3 query steps with a real audio file.
3. **The id-800 meaning and contents.** The value 0x434200 is stable across
   three launches (§5.3); its meaning (the seed marks 800 as 5.0+) and the
   function behind it are unmeasured. **Settling capture:** the in-process
   caller of §8.3 calling the pointer and reading what it is.
4. **The `0x0402` end-of-track behaviour ([S09] U-1).** The post-list probe was
   necessarily a no-op (§7.1). **Settling capture:** a **mid-list probe** — set
   121 to entry 0 of a loaded list, post 0x0402, capture the main window; a
   change there is the positive result. A playing-state variant (post while
   playing a short file) settles the "while playing" half.
5. **The PE-window effects of 101 and 121.** The B3 main-window 0 px results
   are an artefact (101) or ambiguous (121) (§7). **Settling capture:** a job
   that captures the `Winamp PE` window (class `Winamp PE`) before/after
   `IPC_DELETE` and `IPC_SETPLAYLISTPOS`; for 121, a setpos-then-play variant.
6. **The never-probed pointer ids 334, 335, 505 (and 336).** No shape reading
   exists. **Settling capture:** shape probes (expect_reply, asserting a
   nonzero) for 334/335/505/336, then the in-process caller for the contents
   (§8.3).
7. **The meaning of the id-502 reply (1), and 242's network dependence.** 502:
   unsupported vs support-present — 260 returned a real video window handle in
   the same runs; **settling capture:** a job that toggles the video window
   state (or loads a video file) and re-reads 501/502. 242: the reply is
   sandbox-networking-dependent and must never be exported as a fixture
   property; **settling capture:** a no-network sandbox run reading 242.
8. **Window-handle and menu-handle ownership in our host (260, 281).** The
   reference exposes its real windows and a real menu handle (per-boot
   values); how *our* host supplies handles valid in the plugin's process is an
   M3 architecture decision, not a 2.95 fact. **Settling:** the M3 design
   decision, then a plugin exercising 260/281 against our host.
9. **`IPC_ISWNDVISIBLE` (261) on hidden windows.** 261 answered 1 for EQ,
   playlist and main in the fresh state; whether the EQ/playlist windows are
   *actually* visible in that state is not established. **Settling capture:** a
   job that hides (or confirms the visibility of) the EQ/PE windows and
   re-reads 261.
10. **`WM_USER+n` for n other than 0 and 2 ([S09] U-1 second half).** Does 2.95
    handle any private message number besides the end-of-track notification?
    **Settling capture:** a sweep of small n (1, 3, 4, …) posted and sent with
    observable-state checks.
11. **The sent set reply (122, 123).** The setter steps were posted by design,
    so what a *sent* set returns — old value or new value (the [IDX] contract
    says "int (old or new value)") — was never read. **Settling capture:** a job
    step that sends 122/123 with `expect_reply` against a published state.
12. **Posted vs sent for the untested command ids.** §5.2 covers the query ids;
    for each command id, a dead posted id must not be read as "the id does not
    exist". **Settling capture:** per-id posted/sent pairs with an observable
    state check (the 0x0402 probe of §13.4 is the template).
13. **The `WM_COPYDATA` forms (100 enqueue, 103 chdir) and the non-pointer
    classic remainder (120, 130, 241–249, 199, 259, 280, 500, 506–510, 532/533,
    and the §12.3 job-step ids).** **Settling capture:** an s10 follow-up job
    batch; pointer forms are shape-only at best (§8.2).
14. **The sprite identities behind the setter pixels (§6).** The bounds are
    measured (volume frame, balance frame, 6-px shuffle/repeat cells); which
    drawn element each is needs the S01/S02 geometry. **Settling:** S01/S02
    cross-reference, not a new capture.

## 14. Run record (evidence this spec draws on)

| Run | Job | Status (verified) | Used for |
|---|---|---|---|
| 20261016-022004 (R1) | `s10-ipc.json` rev 1 | 68/69 ok; `s10_000_getversion` exit 4 (10501 vs expected 8341); run exit 5; gate passed 0 px/18425; 68 manifest rows, 0 replies read | first version measurement; the setter pixel effects (§6); the rev-1 defect that shaped §0.1 |
| 20261016-023349 (R2) | `s10-ipc.json` rev 2 | 40 ok, 29 by-design reply mismatches, 0 skipped; run exit 5; gate passed; 58 measured replies | §3 rows tagged R2; the by-design correction set (§3.2); §5.5, §5.6 |
| 20261016-060342 (R3) | `s10-ipc.json` rev 3 | 63/69 ok, 6 by-design mismatches (the per-boot handle/pointer probes), 0 skipped; run exit 5; gate passed; 58 measured replies (52 manifest + 6 stderr) | the confirmation of every R2 reply; §3.1; the conclusive EQ write (§5.7) |
| 20261016-065637 (B1a) | `s10b-eq-window.json` | step `eq_select_band0` exit 4 (reply 0 from `Winamp EQ`); run exit 3; gate passed | §5.1, §1.3 (the no-forwarding finding) |
| 20261016-070535 (B1) | `s10b-eq-window.json` (revised) | 6/6 ok; run exit 0; gate passed | §5.7 (the drawn proof: 431 px, side-effect-free read-back) |
| 20261016-070243 (B2) | `s10b-posted-vs-sent.json` | 16 steps, 15 clean; `eof_post` `expect_change` failed by design (0 px); run exit 5; gate passed | §5.2, §5.3, §7.1 |
| 20261016-070703 (B3) | `s10b-destructive.json` | 9/10 ok; `del_action`/`pos_action` `expect_change` failed by design (0 px); `restart_action` exit 3 mid-restart; run exit 5; gate passed | §3 (101, 102, 106, 121, 135), §7 |

All seven runs passed the landmark gate (region 0,49,275,67, 0 differing
pixels of 18425), so the guest, the reference copy and the 1:1 drawing were
sound in every run. The reply runs' exit codes are 5 by design: reply
*mismatches* are recorded as step failures, since a mismatch is the
measurement (§0.1). The pixel figures of §5–§7 were offline-recomputed
identically by the lane audit on the runs' own PNGs.

## 15. Sources relied on

- `rustamp-research/drafts/S10-ipc.md` (the measured S10 draft, 848 lines, the
  object of the lane audit)
- `rustamp-research/audits/S10-ipc-replies.md` (R3 interpretation) and
  `rustamp-research/audits/S10-batches.md` (B1/B2/B3 interpretation)
- `rustamp-research/tools/vm-capture/_runs/20261016-{022004, 023349, 060342,
  065637, 070535, 070243, 070703}` (manifests, step stdout/stderr, run
  summaries, captures)
- `rustamp-research/notes/S10-seed.md` ([SEED] claims; the "likely users"
  column is a source-level guess and is not used as evidence)
- `D:\code\Rustamp/specs/S09-plugin-abi.md` (current text: §8.3 version query
  aligned at 0x2905; §8.4/U-6 unsupported-id guess refuted by this spec;
  §8.9/§8.10 transport and time conventions)
- `D:\code\Rustamp/specs/S02-main-window-behaviour.md` (§1: 104 = 1/3/0 read
  back per state; 125 moves by one on next/prev) and the S01 round-2 supplement
  (124 = 8 for an 8-entry list)
- `D:\code\Rustamp/docs/rfc/0009-ipc-version-constant.md` (the constant
  correction and its pending in-process cross-check)
- `D:\code\Rustamp/crates/rustamp-plugin-abi/src/common.rs`,
  `D:\code\Rustamp/crates/rustamp-ipc/src/message.rs`,
  `D:\code\Rustamp/crates/rustamp-plughost/src/ipc.rs`,
  `D:\code\Rustamp/docs/HANDOFF.md` (2026-10-16: RFC 0009 shipped, plughost
  e2e with thirteen plugin-shaped queries, the i686 sign-extension fix)
- `D:\code\rustamp-lanes/s10x/audits/S10-ipc-audit.md` (the lane audit pass:
  per-id verification, E1–E9, the S09 contradiction rulings)
