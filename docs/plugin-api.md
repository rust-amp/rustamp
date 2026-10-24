# The classic-plugin API guide: writing an `in_`/`out_`/`vis_`/`dsp_`/`gen_` plugin for Rustamp

Written for: authors of classic Winamp 2.x plugins (`in_`, `out_`, `vis_`, `dsp_`,
`gen_`) who want to know what a Rustamp host expects from their DLL — the struct
layouts, export names, call order, lifetimes, threading and units of the classic
ABI.

Companion documents:

| Document | What it is |
|---|---|
| [docs/notes/plugin-authors.md](notes/plugin-authors.md) | The **licensing** guide: under what terms your plugin runs with Rustamp. This document is its named follow-up. |
| [specs/S09-plugin-abi.md](../specs/S09-plugin-abi.md) | The audited ABI specification this guide is written from. |
| `crates/rustamp-plugin-abi` | The `#[repr(C)]` Rust types for the ABI, with compile-time layout asserts. `MIT OR Apache-2.0`; you may reuse it in your own tools. |
| [CLEANROOM.md](../CLEANROOM.md) | The project's binding rules on sources. |

Nothing in this guide is taken from Winamp source, SDK headers, SDK
documentation, disassembly or the Winamp binary. Every ABI statement below
traces to the audited specification `S09-plugin-abi.md` or to the
`rustamp-plugin-abi` crate. Statements that go beyond those two are marked
**[provisional]**, and unsettled points are marked **[open]** with the ID they
carry in S09 §12 (`U-n`) or in
`crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` (`OQ-n`).

---

## 1. What this guide is, and what it is not

**It is** the per-kind author's reference: the module struct and its fields, the
export name and the signature the host calls it with, the call order
(init/quit, config, play/pause notifications, the visualization callback and its
threading), ownership and lifetime rules, the units at the boundary, and what
the host guarantees.

**It is not:**

- **Not legal advice and not the licensing guide.** That is
  [docs/notes/plugin-authors.md](notes/plugin-authors.md).
- **Not the IPC guide (S10) or the embed-window guide (S11).** S09 §8 lists the
  host behaviours classic plugins rely on — the `Winamp v1.x` main-window class,
  the `WM_WA_IPC` channel, `WM_COMMAND` ids, the INI/plugin-directory IPCs —
  but the full catalog is deferred to S10, and window embedding to S11. As of
  the handoff, S10 and S11 are **not started**, so this guide can only point at
  them.
- **Not a promise that a given plugin runs today.** The plugin host
  (`rustamp-plughost32.exe`) is milestone M3 work and, per the handoff, the
  crate is still a stub. The ABI types exist and are pinned; the host that
  exercises them is being built.
- **Not a native plugin API.** The classic ABI is 32-bit x86 Windows only. A
  64-bit or non-Windows build of Rustamp cannot load such a DLL in-process;
  Rustamp solves that by loading your DLL in a separate 32-bit process
  (`rustamp-plughost32.exe`), never in `rustamp.exe` (plugin-authors.md §2).
  A native Rustamp plugin API would be a different, undesigned thing.

---

## 2. Where the facts come from, and which source is authoritative

Two sources are in play, and they are not in competition:

1. **`specs/S09-plugin-abi.md` is authoritative for the ABI** — offsets, sizes,
   field order, export names, version constants, call order and behaviour. It is
   an audited clean-room spec (status: *audited – static; runtime confirmation
   pending*). Where this guide states an ABI fact, S09 is the source.
2. **`crates/rustamp-plugin-abi` is authoritative for how *we* spell those facts
   in Rust.** It is a mechanical restatement of S09: every struct is
   `#[repr(C)]`, every callback is `extern "C"` (cdecl), and **116 compile-time
   layout asserts** pin each size and field offset under
   `#[cfg(target_arch = "x86")]`. The asserts encode exactly S09's numbers and
   add no ABI fact of their own. What the crate *does* add are **modelling
   choices** — the concrete Rust types (`c_int`, `*mut c_void`, `*const c_char`,
   `Option<unsafe extern "C" fn …>`) — and those choices are the ones the crate's
   own `OPEN_QUESTIONS.md` lists as unsettled where S09 is silent. This guide
   flags them rather than asserting them.

If the two ever appear to disagree, S09 wins and the crate has a bug.

**Status of the ABI itself.** S09 is *audited static*: layouts were checked in
the static data of real 2.95 plugins, but the call order, threading and several
numerical conventions are marked `[5X]` (observed in a 5.x host, most likely the
same in 2.95, unconfirmed) or `[UNV]` (unverified). Those tags are carried
through here: where a statement is only a 5.x observation, this guide says so.
S09's version of record is 2.95; where 5.x behaves differently, the 5.x-only
items are collected in §11.

**Provenance tags used below**

| Tag | Meaning |
|---|---|
| *(S09 n.n)* | A fact from the audited spec, section n.n. |
| *(crate)* | A fact encoded by the `rustamp-plugin-abi` crate (types, constants, asserts). |
| **[provisional]** | Goes beyond S09 and the crate: an implementation recommendation or an inference. Not an ABI fact. |
| **[open: U-n / OQ-n]** | Unsettled. Do not build on it. |

---

## 3. The contract common to every kind

These hold for all five kinds (S09 §§0–2).

- **32-bit x86 Windows.** The layouts are exact for a 32-bit x86 process.
  Integers are 32 bits unless stated; pointers, window handles and module
  handles are 32 bits; every function-pointer member has the same width. There
  is **no padding anywhere** in the structures and every member is 4-byte
  aligned, so a C compiler's default packing is correct.
- **cdecl everywhere.** Every callback inside the structures and every
  entry-point export uses cdecl (caller cleans the stack). Exports are
  **undecorated C names**: no leading underscore, no `@n` suffix. In Rust that
  is `extern "C"` (crate: all callbacks are typed `unsafe extern "C" fn …`).
- **One export per DLL, found by name.** Each kind exports exactly one
  "get header or module" function whose name identifies the kind. The host finds
  it by name lookup in the loaded module and looks only for the names of the
  kind it is scanning for. The entry point takes no arguments in 2.x; a 5.x
  host passes the main-window handle to `winampVisGetHeader` and
  `winampDSPGetHeader2`, which under cdecl is a harmless extra argument
  **[open: U-9]**.
- **The returned struct is static and stable.** The struct lives in your DLL's
  static data and must stay valid and at a stable address for as long as the DLL
  is loaded. The host writes several members **in place**, so do not relocate or
  rebuild the struct after the host has seen it. (S09 §0, §2.)
- **Version first.** The first member of every header/module is an integer
  version, and the host refuses a plugin whose version it does not accept. Write
  the 2.x value for your kind (§4 table); the acceptance rule — strict equality
  or a range — is **[open: U-7]**. The crate's `PluginKind::expected_version()`
  returns the 2.x values (crate).
- **Host-filled members are the host's.** Members marked *host* in the tables
  below are written by the host; do not write them yourself. The host fills the
  parent/main window and the instance handle before `init`, and the host-callback
  slots (input only) before `init` too.
- **Strings** are NUL-terminated single-byte strings in the system ANSI code
  page. Wide-character variants exist only in 5.x-era hosts and plugins.
- **Where plugins live.** A `Plugins` sub-folder of the Winamp folder; 2.95
  keeps the folder name in a setting called `PluginDir`, and the string
  `Plugins` is present in the host (S09 §1). **Where Rustamp looks for plugin
  DLLs is not stated in the sources this guide may use** **[open]**, and it is
  not the same question as the name of the folder (plugin-authors.md §2 only
  says plugins are user-installed and Rustamp ships none).
- **Config and About.** Called when the user presses the corresponding button on
  the Plug-ins page of Preferences, on the UI thread. The argument is the parent
  window handle, or the module pointer, depending on kind (see the per-kind
  tables).
- **Unload behaviour.** Input, output and general-purpose DLLs are **not freed**
  at shutdown (plugins may have subclassed windows). Visualization and DSP DLLs
  **are freed** after their `quit` (S09 §1, observed in 5.x).
- **Exception isolation.** The 5.x host wraps visualization `init`/`render`/
  `quit` (and some other quit calls) in structured exception handlers and shows
  a numbered plugin-error dialog on a fault; unknown for 2.95 (S09 §1). Rustamp
  additionally contains a crash by hosting the DLL in a separate process
  (plugin-authors.md §2) — a robustness property, not a compatibility promise.
- **5.x-only, optional for you:** the uninstall export `winampUninstallPlugin`
  (instance handle, preferences window handle, int; return values differ per
  kind — input and general purpose 1 = uninstall now, 0 = after restart; DSP
  0 = now, 1 = after restart; visualization 0 = now; output only supports
  restart, 0) and the safe-mode description-string naming convention. Neither is
  in 2.95 (S09 §1, §10). If you target a 2.95-level host, do not depend on them.

### 3.1 Quick reference

| Kind | File name pattern | Export | Returns | Version | Struct size |
|---|---|---|---|---|---|
| Input | `IN_*.DLL` | `winampGetInModule2` | pointer to static `In_Module` | `0x100` | 152 bytes (`0x98`) |
| Output | `OUT_*.DLL` | `winampGetOutModule` | pointer to static `Out_Module` | `0x10` | 80 bytes (`0x50`) |
| Visualization | any `*.DLL` probed for the export **[open: U-5]** | `winampVisGetHeader` | pointer to static `winampVisHeader` | `0x101` | header 12 bytes; module 2360 bytes (`0x938`) |
| DSP | `DSP_*.DLL` | `winampDSPGetHeader2` | pointer to static `winampDSPHeader` | `0x20` | header 12 bytes (2.x); module 32 bytes |
| General purpose | `GEN_*.DLL` | `winampGetGeneralPurposePlugin` | pointer to static `winampGeneralPurposePlugin` | `0x10` | 28 bytes |

The names above are the public names: the export names and the five
header/module type names are part of the contract and are what plugins and tools
depend on (S09 §0). The **member** names in this guide are descriptive: only the
order, size and offset of each member matter to the ABI. The
`rustamp-plugin-abi` crate happens to use exactly the names below, in snake_case
(crate), but a C plugin may name its fields anything.

Two details worth memorising:

- **Visualization discovery is not a `vis_` prefix filter.** S09 §2 records that
  the 2.95 host's string table contains no `vis_` prefix filter, so **any**
  `*.DLL` in the Plugins folder may be probed for `winampVisGetHeader`
  **[open: U-5]**; the `vis_` file prefix is a convention, not a rule the host
  enforces. The other four kinds are matched by their file-name pattern
  (`IN_`, `OUT_`, `DSP_`, `GEN_`).
- **At most 32 output modules are listed** (5.x), and in 5.x the input plugins
  are loaded in a fixed preferred-name order (the MP3 plugin first, plugins with
  very large extension lists last), then the rest in file enumeration order;
  2.95 ordering is not known **[open]**. Do not depend on load order.

---

## 4. The types we ship (`rustamp-plugin-abi`)

You do not have to use our crate — a classic plugin is a DLL exporting a C
symbol, and you can write it in C, C++ or Rust. But the crate is
`MIT OR Apache-2.0` (plugin-authors.md §3) and it holds the ABI as pinned,
asserted types, which makes it the cheapest way to get the layout right.

| Module | Types |
|---|---|
| `input` | `InModule` (0x98), `InModuleExt` (0x9C, 5.x-only tail) |
| `output` | `OutModule` (0x50) |
| `vis` | `VisHeader` (12), `VisModule` (0x938) |
| `dsp` | `DspHeader` (12), `DspHeader5x` (16), `DspModule` (32) |
| `general` | `GeneralPlugin` (28) |
| `common` | `Hwnd`, `Hinstance`, all version constants, flag bits, export-name byte strings, window class names, `WM_WA_MPEG_EOF`, IPC numbers, `VIS_ROW_LEN = 576`, `VIS_MAX_ROWS = 2`, `read_c_string_lossy` |
| crate root | `PluginKind` with `export_name()`, `from_export_name()`, `expected_version()`, `file_prefix()` |

Every callback member is an `Option<unsafe extern "C" fn …>` (crate), i.e. the
type admits null — see §11 for what that does and does not mean.

An illustrative fragment, **ours**, from `rustamp-plugin-abi::vis`
(`MIT OR Apache-2.0`), restating S09 §5.1 — not SDK text and not copied from
anywhere:

```rust
// ours, from `rustamp-plugin-abi` (MIT OR Apache-2.0)
#[repr(C)]
pub struct VisHeader {
    pub version: c_int,                                       // 0x00
    pub description: *const c_char,                           // 0x04
    pub get_module: Option<unsafe extern "C" fn(index: c_int) -> *mut VisModule>, // 0x08
}
```

**Bitness caveat (crate).** The layout asserts run only on
`target_arch = "x86"`. On a 64-bit target the types merely have to compile;
their 64-bit layout is unused, because the plugin host is 32-bit only. So if you
build your plugin in Rust, build it for `i686-pc-windows-msvc`.

---

## 5. Input plugins (`in_`)

An input plugin opens a file or stream, decodes it, and hands audio to the
player. It also drives the output plugin, the DSP/EQ hooks and the
analyzer/visualization feed (S09 §3.5).

### 5.1 Entry point

| | |
|---|---|
| Export | `winampGetInModule2` (crate: `common::EXPORT_IN_GET_MODULE`) |
| Arguments | none |
| Returns | pointer to your static `In_Module` |
| Version | `0x100` |
| Size | 38 members, 152 bytes (`0x98`) |

Host side (S09 §3.5, step 1): load the DLL, look up the export, call it with no
arguments, receive the module pointer, verify the version. A 2.95-level host
should accept exactly `0x100` **[open: U-7]**; the `0x101`, `0x0F000000` and
`0xF0000000` variants are 5.x-only (S09 §3.2).

### 5.2 `In_Module`, field by field

Offsets are S09 §3.1; the crate asserts every one of them (crate).

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `version` | int | plugin | `0x100` for all 2.95 plugins. |
| 1 | 0x04 | `description` | pointer to ANSI string | plugin | Human-readable name and version for the Plug-ins list. |
| 2 | 0x08 | `main_window` | window handle | host | Winamp's main window; set before `init`. |
| 3 | 0x0C | `dll_instance` | module handle | host | Your own module handle, for resource lookup. |
| 4 | 0x10 | `file_extensions` | pointer to double-NUL-terminated string list | plugin | Pairs of strings: extension list, then display name. May be rewritten by you at run time from `config`. |
| 5 | 0x14 | `seekable` | int (0/1) | plugin | Whether the current stream is seekable. May change while playing. |
| 6 | 0x18 | `flags` | int (bit flags) | plugin | Bit 0: audio goes through a Winamp output plugin. 2.95 plugins hold 1 or 0. |
| 7 | 0x1C | `config` | fn(parent window handle) | plugin | Open the configuration dialog. |
| 8 | 0x20 | `about` | fn(parent window handle) | plugin | Open the about box. |
| 9 | 0x24 | `init` | fn() | plugin | Called once at startup, after the host fields are set. Returns nothing in 2.x; 2.x ignores any return value. |
| 10 | 0x28 | `quit` | fn() | plugin | Called once at exit. |
| 11 | 0x2C | `get_file_info` | fn(ANSI path or null, title buffer, pointer to int length-in-ms) | plugin | Fill title and length for a file. A null path means the file currently playing. The title buffer holds **2048 bytes**. Length `-1` means unknown or stream. |
| 12 | 0x30 | `info_box` | fn(ANSI path, window handle) -> int | plugin | Show the file-info dialog. Returns 0 if the tags were edited, 1 if unchanged. |
| 13 | 0x34 | `is_our_file` | fn(ANSI name) -> int | plugin | Called **before** extension matching, so you can claim non-file names (URLs, special schemes). Nonzero = this plugin handles it. |
| 14 | 0x38 | `play` | fn(ANSI path) -> int | plugin | Start playback. 0 = success, -1 = file not found, any other nonzero = another error that stops the playlist advance. |
| 15 | 0x3C | `pause` | fn() | plugin | Pause. |
| 16 | 0x40 | `unpause` | fn() | plugin | Resume. |
| 17 | 0x44 | `is_paused` | fn() -> int | plugin | 1 if paused, 0 otherwise. |
| 18 | 0x48 | `stop` | fn() | plugin | Stop and unload the stream. You must stop your decode thread and close the output (if used) before returning. |
| 19 | 0x4C | `get_length` | fn() -> int | plugin | Track length in ms. |
| 20 | 0x50 | `get_position` | fn() -> int | plugin | Current playback position in ms; normally the output module's played time. May be called from threads other than the UI thread. |
| 21 | 0x54 | `seek` | fn(int ms) | plugin | Seek. Normally signals the decode thread to seek and then flushes the output at the new time. |
| 22 | 0x58 | `set_volume` | fn(int) | plugin | 0..255; normally forwarded to the output module. |
| 23 | 0x5C | `set_pan` | fn(int) | plugin | **-127..127** on the input side (output plugins use -128..128); normally forwarded to the output module. |
| 24 | 0x60 | `analyzer_init` | fn(int max_latency_ms, int sample_rate) | **host** | Call once in `play`, after the output is open, passing the latency returned by the output's `open` (or 0) and the sample rate. Prepares the host's analyzer and visualization buffering. |
| 25 | 0x64 | `analyzer_deinit` | fn() | **host** | Call from `stop`. |
| 26 | 0x68 | `analyzer_feed_pcm` | fn(pointer to PCM, int channels, int bits_per_sample, int timestamp_ms) | **host** | Easy path for the classic in-window analyzer: hand over interleaved PCM and the host computes spectrum/scope itself. **At least 576 sample frames per call.** |
| 27 | 0x6C | `analyzer_get_mode` | fn() -> int | **host** | Advanced path: which analyzer data the host wants. Bit 0 (1) spectrum, bit 1 (2) oscilloscope, value 4 none/window-shade; 3 = both. |
| 28 | 0x70 | `analyzer_feed_rows` | fn(pointer to data, int timestamp_ms, int mode) -> int | **host** | Advanced path: you supply precomputed analyzer rows for the given mode value. Nonzero return = host is not collecting data. |
| 29 | 0x74 | `vis_feed_pcm` | fn(pointer to PCM, int channels, int bits_per_sample, int timestamp_ms) | **host** | Easy path for visualization plugins; same arguments as `analyzer_feed_pcm`, also **at least 576 sample frames per call**. |
| 30 | 0x78 | `vis_get_mode` | fn(pointer to int spectrum_rows, pointer to int waveform_rows) -> int | **host** | Advanced path: how many spectrum and waveform rows (0, 1 or 2 each) the running visualization wants. Returns nonzero when a visualization is running; both outputs are 0 otherwise. |
| 31 | 0x7C | `vis_feed_frame` | fn(pointer to data, int timestamp_ms) -> int | **host** | Advanced path: supply one frame laid out as the spectrum rows followed by the waveform rows, **576 bytes per row**. Nonzero return = not accepted. |
| 32 | 0x80 | `vis_set_format` | fn(int sample_rate, int channels) | **host** | Call once in `play` to announce the stream format to the visualization. **Argument order is sample rate, then channels.** |
| 33 | 0x84 | `dsp_is_active` | fn() -> int | **host** | Whether the host will modify samples. 0 = no. Nonzero = yes, and the output frame count may exceed the input count, so size your buffer for up to **twice** the input. |
| 34 | 0x88 | `dsp_process` | fn(pointer to 16-bit signed samples, int frames, int bits_per_sample, int channels, int sample_rate) -> int | **host** | Runs the active DSP plugin and EQ over the block **in place**. Returns the number of frames now in the buffer (up to double the input). |
| 35 | 0x8C | `eq_set` | fn(int on, pointer to 10 band bytes, int preamp) | plugin | **Optional.** A plugin with its own EQ implements it; the host calls it when EQ settings change. Values 0..63: 31 = 0 dB, 0 = +12 dB, 63 = -12 dB. A plugin without its own EQ leaves a **stub that ignores calls**. |
| 36 | 0x90 | `set_info` | fn(int bitrate_kbps, int sample_rate_khz, int channels, int synched) | **host** | You report stream properties for the main-window readouts. `-1` means "no change" per argument. Sample rate is in **kHz** (e.g. 44), channels is the count, `synched` is a 0/1 flag. |
| 37 | 0x94 | `out_module` | pointer to `Out_Module` | **host** | Selected output plugin; set by the host before `play` when flag bit 0 is set. |

Total `0x98`. S09 §3.1 records that the host fills members 2, 3, 24–34, 36 and
37 (in the static data of the 2.95 plugins examined those read as zero, while
member 35 held a code pointer).

**5.x-only tail.** A 39th member (pointer to the host service table, offset
`0x98`) exists only if the version value carries the `0xF0000000` "init returns
a status" flag (S09 §3.1, §3.2). The crate models this as a separate
`InModuleExt` with `base: InModule` and `host_services: *mut c_void` (crate;
`OQ-7`). For a 2.95-level plugin, do not use it.

### 5.3 Call order

**Host start** (S09 §3.5, main thread):

1. Load the DLL, call `winampGetInModule2`, verify the version.
2. Write `dll_instance`, `main_window`, and the host-provided members:
   `dsp_process`, `dsp_is_active`, `analyzer_get_mode`, `analyzer_feed_rows`,
   `analyzer_init`, `analyzer_deinit`, `vis_set_format`, `vis_feed_frame`,
   `vis_get_mode`, `analyzer_feed_pcm`, `vis_feed_pcm`.
3. **`set_info` is cleared and installed only after `init` returns.** During
   `init` it is not yet the host's function — do not call it there.
4. Call `init()`, then add the module to the list.

**Per play request** (S09 §3.5, main thread):

1. The host chooses a module: a first pass calls every module's `is_our_file` in
   list order; a second pass matches extension lists in list order.
2. If flag bit 0 is set, the host picks the output named by `config_outname`,
   falls back to DirectSound then waveOut, and aborts with an error if none
   exists. It sets `out_module` (and the output's main window), then applies
   volume and pan.
3. The host calls `play(path)`. **You** then: open the file, call the output's
   `open`, then `analyzer_init(latency, rate)`, `vis_set_format(rate, channels)`
   and `set_info(...)`, and start your decode thread.
4. **Decode thread loop:** decode → optionally `dsp_is_active` / `dsp_process` →
   `vis_feed_pcm` / `analyzer_feed_pcm` → output `can_write` / `write`.
   Timestamps come from the output's written time.
5. **End of file:** wait for the output's `is_playing` to go false, then post the
   end-of-track notification (below).
6. **User stop:** the host calls `stop()`; you stop your decode thread, close the
   output and call `analyzer_deinit()`.

**Host exit** (main thread): `quit()` is called on every loaded input module in
**reverse load order**. Your DLL is not freed.

The order above is `[5X]` and matches the documented usage; the layout and the
presence of the slots are `[295]`-confirmed (S09 §3.5).

### 5.4 End-of-track notification

When a track ends on its own, your decode thread **POSTS** (not sends) window
message number **`0x0402`** (`WM_USER + 2`, public name `WM_WA_MPEG_EOF`) to the
main window with **both parameters zero**. Post it only if you were not asked to
stop. On receipt the host advances to the next playlist item or stops. (S09 §3.6;
that the host receives it this way in 2.95 is **[open: U-1]**.)

This is **not** the IPC channel: IPC uses message number `0x0400` with the command
in the second parameter, while this notification uses `0x0402` itself, so a host
must handle it as a separate message (S09 §3.6).

### 5.5 Threading

| Call | Thread (S09 §3.6) |
|---|---|
| `init`, `quit`, `config`, `about`, `info_box`, `is_our_file` | UI thread |
| `play`, `stop`, `pause`, `unpause`, `seek`, `set_volume`, `set_pan`, `eq_set` | UI thread; must not block for long |
| `get_position`, `get_length`, `is_paused` | UI thread **plus** the host's classic-analyzer thread and the visualization thread (the vis loop polls the position every frame) — **must be thread-safe** |
| `get_file_info` | UI thread, possibly worker threads when playlists are built **[open]** |
| Output `write`/`can_write`/`is_playing`/`open`/`close` | your decode thread; flush from the seek path |
| Analyzer and vis feed calls, `dsp_process` | your decode thread |

The 2.95 thread model is partly unverified (S09 §12 U-2; HANDOFF open point 6).
Treat every callback that the table says can arrive from more than one thread as
concurrent.

### 5.6 Ownership and lifetimes

- The struct is **yours**, static, stable address; the host writes into members
  2, 3, 24–34, 36, 37 in place (S09 §0, §3.1). Never relocate it.
- `description` and `file_extensions` point at **your** memory and must stay
  valid while the DLL is loaded. `file_extensions` may be rewritten at run time
  (S09 §3.4). Whether the pointer may be null in static data is **[open:
  U-19]** — the two plugins examined held a non-null pointer, so supply one.
- The **title buffer** in `get_file_info` is the **caller's** 2048-byte buffer
  (S09 §3.1). Fill it; do not keep the pointer.
- The buffers the host passes to the feed calls (`analyzer_feed_pcm`,
  `analyzer_feed_rows`, `vis_feed_pcm`, `vis_feed_frame`, `dsp_process`) are the
  caller's. S09 does not state how long they stay valid **[provisional]**: treat
  them as valid only for the duration of the call and copy anything you need to
  keep.
- `out_module` is the host's pointer to the selected output module. Use it, do
  not free it, and do not cache it across plays — the user may change the output
  between plays (S09 §3.7).
- Your DLL is **not freed** at shutdown (S09 §1), but `quit` is the last call you
  are promised; release what `init` and `play` acquired.
- The host owns the playlist advance: `play`'s return value decides whether the
  playlist moves on (S09 §3.1, member 14).

**Which callbacks may be null.** S09 does not settle which input callbacks a
host tolerates as null **[open: U-16]**; the plugins examined fill every slot, and
the crate models every callback as an `Option` (crate `OQ-16`). **Fill every
slot.** The one documented exception is `eq_set`: a plugin without its own EQ
"leaves a stub that ignores calls" (S09 §3.1, member 35) — a stub, not a null
pointer, is what was observed.

### 5.7 Units on the input side

| Quantity | Unit |
|---|---|
| `get_length`, `get_position`, `seek` | milliseconds |
| `set_volume` | 0..255 |
| `set_pan` (input side) | -127..127 |
| `set_info` bitrate | kbps |
| `set_info` sample rate | **kHz** (e.g. 44) |
| `get_file_info` length | ms; `-1` = unknown or stream |
| `eq_set` band values | 0..63, 31 = 0 dB, 0 = +12 dB, 63 = -12 dB |
| `analyzer_init` latency | milliseconds (the output's `open` return, or 0) |
| `analyzer_init` sample rate | S09 does not state the unit **[provisional]**: Hz, by analogy with `vis_set_format` and the vis module's `sample_rate` field, which is Hz |
| feed-call timestamps | milliseconds, from the output's written time |

### 5.8 A minimal input plugin, in prose

1. Put one `In_Module` instance in your DLL's static data, zeroed except the
   fields you own. It must stay at a stable address for the DLL's life.
2. Set `version` to `0x100`, point `description` at a NUL-terminated ANSI string
   that lives as long as the DLL, and point `file_extensions` at a
   double-NUL-terminated list of (extension list, display name) pairs.
3. Set `flags`: bit 0 if your audio goes through the selected output module.
4. Fill every callback slot with a cdecl function of the signature in §5.2. Give
   `eq_set` a stub that ignores its arguments if you have no EQ of your own.
5. Export `winampGetInModule2` as an undecorated C function taking no arguments
   and returning the pointer to that instance.
6. In `init`, prepare what you need. Do **not** call `set_info` — the host
   installs it only after `init` returns.
7. In `play`: open the file, call the output's `open` (when flag bit 0 is set),
   then `analyzer_init(latency, rate)`, `vis_set_format(rate, channels)` and
   `set_info(...)`, then start your decode thread.
8. In the decode thread: per block, optionally ask `dsp_is_active` and call
   `dsp_process`, then feed the visualization/analyzer (`vis_feed_pcm` /
   `analyzer_feed_pcm`, at least 576 frames per call), then write to the output
   via `can_write` / `write`.
9. At the end of a track that ends by itself, wait for the output's `is_playing`
   to go false, then post `0x0402` to `main_window` with both parameters zero.
10. In `stop`: stop your decode thread, close the output, call `analyzer_deinit`.
11. In `quit`: release everything `init` acquired. Your DLL is not freed
    afterwards, but no further call is promised.

---

## 6. Output plugins (`out_`)

An output plugin receives PCM from an input plugin and plays it (or writes it to
disk). The input plugin drives it; the host never calls `write` itself.

### 6.1 Entry point

| | |
|---|---|
| Export | `winampGetOutModule` |
| Arguments | none |
| Returns | pointer to your static `Out_Module` |
| Version | `0x10` (`0x11` is 5.x-only, wide description) |
| Size | 20 members, 80 bytes (`0x50`) |

### 6.2 `Out_Module`, field by field

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `version` | int | plugin | `0x10` in 2.95. |
| 1 | 0x04 | `description` | pointer to ANSI string | plugin | Name for the output list. |
| 2 | 0x08 | `id` | 32-bit int | plugin | Module identifier. 2.95 plugins carry large fixed constants; **third-party modules should use values >= 65536**. |
| 3 | 0x0C | `main_window` | window handle | host | |
| 4 | 0x10 | `dll_instance` | module handle | host | |
| 5 | 0x14 | `config` | fn(parent window handle) | plugin | |
| 6 | 0x18 | `about` | fn(parent window handle) | plugin | |
| 7 | 0x1C | `init` | fn() | plugin | Called when the output list is built at startup. |
| 8 | 0x20 | `quit` | fn() | plugin | Called at exit. |
| 9 | 0x24 | `open` | fn(int sample_rate, int channels, int bits_per_sample, int buffer_ms, int prebuffer_ms) -> int | plugin | Open the device. Returns the **maximum latency in ms** (0 for disk writers) or a **negative value on failure**. Buffer arguments are hints (0 = default, prebuffer <= buffer) that most plugins ignore. |
| 10 | 0x28 | `close` | fn() | plugin | Close the device. |
| 11 | 0x2C | `write` | fn(pointer to bytes, int length) -> int | plugin | **Non-blocking.** Length is **at most 8192 bytes** per call. Returns 0 when the data was taken, nonzero when it could not be taken yet. |
| 12 | 0x30 | `can_write` | fn() -> int | plugin | Bytes that can be written right now. Does not shrink except through `write` or `close`. |
| 13 | 0x34 | `is_playing` | fn() -> int | plugin | Nonzero while buffered audio is still audible. The input plugin must wait for 0 before `close`, or the end of the song is cut off. |
| 14 | 0x38 | `pause` | fn(int state) -> int | plugin | 1 pauses, 0 resumes; **returns the previous state**. |
| 15 | 0x3C | `set_volume` | fn(int) | plugin | 0..255. |
| 16 | 0x40 | `set_pan` | fn(int) | plugin | -128..128. |
| 17 | 0x44 | `flush` | fn(int time_ms) | plugin | Drop buffered audio and restart the clock at the given time (seeking). |
| 18 | 0x48 | `get_played_ms` | fn() -> int | plugin | Milliseconds actually played, counted from the last flush time (or 0). |
| 19 | 0x4C | `get_written_ms` | fn() -> int | plugin | Milliseconds written so far — **this is what visualization timestamps are built from**. |

Total `0x50`. In the 2.95 plugins examined, the two host-filled members
(`main_window`, `dll_instance`) read as zero in static data.

Two special values in `open`:

- `buffer_ms == -666` means the output clock will be used to sync video; the
  DirectSound output then disables silence trimming (S09 §4.1).
- `prebuffer_ms <= buffer_ms`; `0` means "default".

**The `id` member is not stable.** S09 §4.1 notes that the 5.x host **overwrites
`id`** with a pointer to a copy of the DLL file name and matches the saved
selection through it. The crate keeps it as raw 32 bits for that reason (crate
`OQ-1`). A 2.95-level host is not known to do this **[open: U-4]**; either way,
do not read `id` back after the host has touched the module, and do not build
behaviour on its value.

### 6.3 Call order

1. The host loads each `OUT_*.DLL`, calls `winampGetOutModule`, checks the
   version, writes `dll_instance` and `main_window` (in 5.x also replaces `id`),
   and calls `init` (S09 §4.2).
2. The user picks the active output in Preferences; the choice is stored **by
   plugin file name** in the setting `config_outname` (S09 §4.2).
3. Per playback the **input plugin** calls `open`, then `write` / `can_write` in
   a loop from its own thread, then waits on `is_playing`, then `close`.
   `pause`, `set_volume`, `set_pan` and `flush` come from the UI or the seek path
   through the input plugin (S09 §4.2).
4. At exit the host calls `quit` on all outputs and does **not** free the DLLs
   (S09 §4.2).
5. **5.x-only:** the optional export `winampGetOutModeChange(int mode)` tells a
   plugin when it becomes active or inactive: 0 = no longer active, 1 = becoming
   active, `0x100` OR-ed in when the change is triggered by playback start. Not
   in 2.95 (S09 §4.2, §10).

### 6.4 Threading

`open`, `close`, `write`, `can_write`, `is_playing` and `get_written_ms` are
called from the input plugin's **decode thread**; `get_played_ms` may also come
from the **UI timer thread** and the **visualization thread**; `pause`,
`set_volume`, `set_pan` and `flush` come from the **UI or seek path**.
Implementations must be thread-safe (S09 §4.2).

### 6.5 Ownership and lifetimes

- The struct is yours, static, stable; the host writes `main_window` and
  `dll_instance` in place (and, in 5.x, `id`). Do not relocate it.
- The `write` buffer is the **input plugin's**; you receive a pointer and a
  length and must not keep either beyond the call. Copy what you buffer.
- `can_write` must not grow spontaneously — it "does not shrink except through
  `write` or `close`" (S09 §4.1), which is the contract the input plugin's loop
  is written against.
- Your DLL is **not freed** at shutdown (S09 §1).
- Which of your callbacks the host tolerates as null is not stated by S09
  **[open: U-16 is about input, but no per-kind nullability rule exists]**;
  the crate models all of them as `Option` (crate). Implement all of them. In
  particular `open` returning negative is how you report failure, so a null
  `open` would be indistinguishable from a broken plugin.

### 6.6 Units

| Quantity | Unit |
|---|---|
| `open` sample rate | Hz |
| `open` channels / bits per sample | count / bits |
| `open` buffer, prebuffer | milliseconds (hints; 0 = default; `buffer_ms == -666` = video sync) |
| `open` return | max latency in ms, 0 for disk writers, negative on failure |
| `write` length | bytes, at most 8192 |
| `can_write` return | bytes |
| `pause` state / return | 1 or 0 / previous state |
| `set_volume` | 0..255 |
| `set_pan` | -128..128 (note: the input side uses -127..127) |
| `flush`, `get_played_ms`, `get_written_ms` | milliseconds |

### 6.7 A minimal output plugin, in prose

1. Put one `Out_Module` in static data; set `version` to `0x10`, point
   `description` at a persistent ANSI string, and set `id` to a fixed constant
   **>= 65536**.
2. Fill every callback slot with a cdecl function of the signature in §6.2.
3. Export `winampGetOutModule` as an undecorated C function taking no arguments
   and returning the pointer to that struct.
4. In `init`, prepare device enumeration. Do not assume a stream format yet — it
   arrives with `open`.
5. In `open(rate, channels, bits, buffer_ms, prebuffer_ms)`: open the device,
   treat the buffer arguments as hints, honour `-666` as "sync to the output
   clock", and return the maximum latency in ms (0 if you write to disk), or a
   negative value on failure.
6. In `write(data, len)`: accept at most 8192 bytes, never block, return 0 when
   taken and nonzero when you cannot take it yet.
7. In `can_write`: report the bytes you can accept right now. The input plugin
   loops on this, so it must be honest.
8. In `is_playing`: report nonzero while buffered audio is still audible — the
   input plugin waits for 0 before `close`.
9. Implement `pause`, `set_volume`, `set_pan`, `flush`, `get_played_ms`,
   `get_written_ms` with the units above. `get_played_ms` counts from the last
   flush time; `get_written_ms` feeds visualization timestamps.
10. In `close`, release the device; in `quit`, release what `init` acquired. Your
    DLL is not freed.

---

## 7. Visualization plugins (`vis_`)

A visualization plugin owns its own window and draws one frame per call from
arrays the host fills.

### 7.1 Entry point and header

| | |
|---|---|
| Export | `winampVisGetHeader` |
| Arguments | none in 2.x (a 5.x host passes the main-window handle; under cdecl the unused extra argument is harmless) **[open: U-9]** |
| Returns | pointer to your static `winampVisHeader` |
| Version | `0x101` (`0x102` goes with the main-window-taking 5.x entry point) |

`winampVisHeader`, 12 bytes (S09 §5.1):

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `version` | int | plugin | `0x101` in 2.x. |
| 1 | 0x04 | `description` | pointer to ANSI string | plugin | Library name. |
| 2 | 0x08 | `get_module` | fn(int index) -> module pointer | plugin | Returns module `index` (0-based) or **null if out of range**. The host calls 0, 1, 2… until null to list the modules; Preferences stores the library file name and the module index. |

The 2.95 AVS plugin keeps the header directly before its first module in static
data, and its `description` and `get_module` members are **zero in the static
image** and filled at run time (S09 §5.1) — a reminder that static
initialisation can be partial as long as the host never sees the struct before
your entry point returns it.

### 7.2 `winampVisModule`, field by field

2360 bytes (`0x938`), S09 §5.2:

| # | Offset | Member | Size | Type | Filled by | Meaning |
|---|---|---|---|---|---|---|
| 0 | 0x000 | `description` | 4 | pointer to ANSI string | plugin | Module name for the Vis list. |
| 1 | 0x004 | `parent_window` | 4 | window handle | host | Winamp main window; set before `init` **and** `config`. |
| 2 | 0x008 | `dll_instance` | 4 | module handle | host | |
| 3 | 0x00C | `sample_rate` | 4 | int | host | Stream sample rate in **Hz**; `44100` before any `vis_set_format`; updated live afterwards. |
| 4 | 0x010 | `channels` | 4 | int | host | Stream channel count; default 2. |
| 5 | 0x014 | `latency_ms` | 4 | int | plugin | How far ahead of "now" you want your data: the host looks for the frame at output position **+ this value**. Seen: 14 (AVS), 0 (MilkDrop). |
| 6 | 0x018 | `delay_ms` | 4 | int | plugin | Pause in ms the host inserts **after** each `render`. Seen: 14 (AVS), 10 (MilkDrop). |
| 7 | 0x01C | `spectrum_rows` | 4 | int | plugin | Spectrum rows wanted: 0, 1 or 2. |
| 8 | 0x020 | `waveform_rows` | 4 | int | plugin | Waveform rows wanted: 0, 1 or 2. |
| 9 | 0x024 | `spectrum_data` | 1152 | 2 x 576 unsigned bytes | host | Valid for the first `spectrum_rows` rows. |
| 10 | 0x4A4 | `waveform_data` | 1152 | 2 x 576 unsigned bytes | host | Valid for the first `waveform_rows` rows. |
| 11 | 0x924 | `config` | 4 | fn(module pointer) | plugin | Configuration dialog. |
| 12 | 0x928 | `init` | 4 | fn(module pointer) -> int | plugin | Create windows and resources. **0 = success**; nonzero aborts the start and the DLL is released. |
| 13 | 0x92C | `render` | 4 | fn(module pointer) -> int | plugin | Draw one frame from the data arrays. **0 = continue**; nonzero ends the visualization (the host then calls `quit`). |
| 14 | 0x930 | `quit` | 4 | fn(module pointer) | plugin | Destroy windows and resources. |
| 15 | 0x934 | `user_data` | 4 | pointer | plugin | Free for your use. |

Unlike the input and general-purpose structs, **every vis callback takes the
module pointer** (S09 §5.2), which is how you reach your own `user_data`.

### 7.3 Call order and cadence

1. The user picks a module in Preferences, from the main window's vis button, or
   via the IPC "execute plugin" message. Winamp stops any running visualization
   first (S09 §5.3).
2. Winamp creates a **dedicated vis thread**. On it the host loads the DLL,
   resolves `winampVisGetHeader`, calls it, calls `get_module(index)`, writes
   `sample_rate`, `channels`, `parent_window` and `dll_instance` into the module,
   then calls `init(module)`. A null header, a null module or a nonzero `init`
   result unloads the DLL and stops (S09 §5.3).
3. After a successful `init` the host sizes its frame ring from your row counts
   (each frame is `(spectrum_rows + waveform_rows) x 576` bytes plus a 4-byte
   timestamp) and, if the user chose to disable the built-in visualization while
   a plugin runs, turns off the classic analyzer (S09 §5.3).
4. **Render loop on the vis thread, until stopped** (S09 §5.3):
   a. If a window message is pending, dispatch it normally. The window the
      plugin registered for embedding receives dialog-style handling; key
      messages for that window are not pre-processed; mouse-wheel events over
      windows of other threads are forwarded to the main window. A quit message
      ends the loop.
   b. Otherwise, **if not paused**: when a track is playing, fetch the frame
      nearest to (input plugin position + `latency_ms`) and copy
      `spectrum_rows` rows then `waveform_rows` rows into the module arrays; if
      no frame is available, or nothing is playing, **zero both arrays
      completely**. Call `render` while a track is playing, and once more right
      after playback stops (a last frame with zeroed data), then not again until
      playback restarts. A nonzero return ends the loop. Finally sleep
      `delay_ms` ms — **render time is not subtracted**, so the real interval is
      render time plus `delay_ms`.
   c. **If paused**: sleep the smaller of 1 ms and `delay_ms`, keep dispatching
      messages, make **no** `render` calls and **no** data refresh.
5. On leaving the loop (nonzero `render`, quit message or user stop): free the
   frame ring, re-enable the classic analyzer per the user setting, call
   `quit(module)` **on the vis thread**, then **free the DLL**. A stop requested
   from another thread sets a kill flag and waits, pumping UI messages, in 10 ms
   steps for up to about 2 seconds, then waits indefinitely. The 2.95 host
   imports a thread-termination API, so it may kill hung vis threads (S09 §5.3).
6. The vis thread priority comes from a five-level user setting (lowest, below
   normal, normal, above normal, highest) named `config_visplugin_priority`
   (S09 §5.3).
7. `config(module)` is called from the **UI thread** when the user presses
   Configure; the module's parent-window member is already valid then. This is
   `[UNV]` for 2.95, and some plugins read the parent window inside `config`
   (S09 §5.3, crate `OQ-14`).

The 2.95 thread model is **[open: U-2]**: whether 2.95 runs visualizations on a
separate thread as 5.x does, and which thread calls `config`, `init` and `quit`,
is unconfirmed.

### 7.4 Data you receive

Each row has exactly **576 entries** (S09 §5.2, §9):

- **Waveform rows** hold signed 8-bit amplitudes (two's complement, 0 =
  silence) in the 5.x host for 16-bit input; the MilkDrop consumer decodes them
  as signed. The 2.95 convention is **[open: U-3]** — the byte array is typed
  `u8` in the crate precisely because the sign is unsettled (crate `OQ-9`).
- **Spectrum rows** hold magnitude bytes 0..255 in 5.x; only the first 512
  entries carry FFT-derived data and the remaining entries are filled with a
  decaying continuation.
- For a **mono source and a plugin asking for 2 rows**, the host duplicates the
  row into the second. If the source has more channels than requested rows, the
  extra channels are dropped; a **mono request averages all channels** (5.x).
- Row 0 is left, row 1 is right (S09 §9; channel order is a 5.x observation).
- A visualization with **no frame available gets zeroed arrays** (S09 §3.8).
- The host fills `spectrum_data` and `waveform_data` for you — you read them in
  `render`. Do not write them.
- The numerical processing of the 576-entry arrays belongs to spec S08, which
  does not exist yet; S09 defers it, and the row encoding of the **classic
  analyzer's** 75-byte rows is explicitly **[open: U-17]**.

### 7.5 Window and settings conventions

- A visualization window with focus should **pass keystrokes through to
  Winamp's main window** so playback stays controllable, except keys you use
  yourself (S09 §5.3).
- Store settings under the INI directory obtained by IPC, in your own file in
  the Plugins folder (S09 §5.3; the IPC catalog is S10).
- **The plugin owns its window.** The host never creates a window for it
  (S09 §5.3). Embedding into a host frame is the S11 embed API, which is not
  written yet.
- **One visualization is active at a time** (S09 §5.3).
- `config_visplugin_autoexec` makes the chosen visualization start when the
  player starts or playback begins (S09 §5.3).
- **5.x-only:** IPC 611 registers a "vis command window" that receives
  next/previous/random/fullscreen/config/menu commands (command ids
  40382..40391); MilkDrop 2.x uses it. Do not depend on it in a 2.95-targeted
  plugin (S09 §5.3, §10).

### 7.6 Ownership and lifetimes

- The header and every module are **yours**, static, stable addresses; the host
  writes `parent_window`, `dll_instance`, `sample_rate` and `channels` into the
  module **in place** (S09 §5.2).
- `user_data` is yours to allocate and free. Nothing in S09 says the host
  touches it; the AVS module's static image has it zero.
- **Your DLL is freed after `quit` returns** (S09 §1, §5.3). That is stricter
  than the input/output/general-purpose case: after `quit` your code and static
  data may be gone, so join or kill your threads inside `quit`.
- `description` strings must stay valid while the DLL is loaded.
- Which callbacks may be null is not settled by S09 for this kind **[open]**.
  The crate models all of them as `Option`; the AVS module's static image has
  host-filled members zero and its four callbacks as code pointers. Implement
  `init`, `render` and `quit` at minimum; `config` is the Configure button, and
  a missing `config` is at best an unusable Preferences button **[provisional]**.

### 7.7 A minimal visualization plugin, in prose

1. Put one `VisHeader` in static data with `version = 0x101`, a persistent
   `description`, and `get_module` pointing at a cdecl function that returns
   your `index`-th module or null.
2. Put one `VisModule` per visualization you offer in static data:
   `description`, `latency_ms`, `delay_ms`, `spectrum_rows` and `waveform_rows`
   (each 0, 1 or 2), the four callbacks, and `user_data` for your own state.
3. Export `winampVisGetHeader` as an undecorated C function taking no arguments
   (for a 2.x-targeted build) and returning the header pointer.
4. In `init(module)`: create your window and resources, stash them through
   `user_data`, and return 0. A nonzero return aborts the start and the DLL is
   released.
5. In `render(module)`: read the first `spectrum_rows` rows of `spectrum_data`
   and then the first `waveform_rows` rows of `waveform_data`, draw one frame,
   and return 0 to continue or nonzero to end. Do not render from a thread of
   your own — the host drives `render` on its vis thread and sleeps `delay_ms`
   after each call.
6. In `quit(module)`: destroy your window and free your resources. The DLL is
   freed as soon as this returns, so stop your threads here.
7. Read `parent_window`, `dll_instance`, `sample_rate` and `channels` from the
   module; treat them as live values the host updates.
8. Never block for long and never call back into the host's own UI from
   `render` **[provisional]** — the loop is single-threaded on the vis thread.

---

## 8. DSP plugins (`dsp_`)

A DSP plugin transforms a block of samples in place between the decoder and the
output.

**Caveat on this kind.** 2.95 ships no DSP plugin in its stock or full
installers, although its host holds the `DSP_*.DLL` pattern and the settings
`config_dspplugin_name` / `config_dspplugin_num`. The layouts below come from the
public headers and the 5.x host and have **not** been checked against a 2.x-era
DSP DLL (S09 §6). Treat the layout as audited-by-provenance, not
checked-in-the-wild.

### 8.1 Entry point and header

| | |
|---|---|
| Export | `winampDSPGetHeader2` |
| Arguments | none in 2.x; the main-window handle in 5.x (a harmless extra argument under cdecl) **[open: U-9]** |
| Returns | pointer to your static `winampDSPHeader` |
| Version | `0x20` (`0x21`/`0x22` are 5.x-only, with the main-window-taking entry point) |

`winampDSPHeader` — **12 bytes in 2.x, 16 bytes in 5.x** (S09 §6.1):

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `version` | int | plugin | `0x20` for 2.x. |
| 1 | 0x04 | `description` | pointer to ANSI string | plugin | |
| 2 | 0x08 | `get_module` | fn(int index) -> module pointer | plugin | Null if out of range. |
| 3 | 0x0C | `key_check` | fn(int key) -> int | plugin | **5.x-only**, absent from 2.x headers. |

The crate models both sizes: `DspHeader` (12) and `DspHeader5x` (16) (crate).
**S09 does not say which size a host should expect for a `0x20` header**
**[open: crate `OQ-8`]**; a 2.x-targeted plugin should use the 12-byte form,
which is what "12 bytes in 2.x" states.

### 8.2 `winampDSPModule`, field by field

32 bytes (S09 §6.2):

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `description` | pointer to ANSI string | plugin | |
| 1 | 0x04 | `parent_window` | window handle | host | Set before `init`. |
| 2 | 0x08 | `dll_instance` | module handle | host | |
| 3 | 0x0C | `config` | fn(module pointer) | plugin | |
| 4 | 0x10 | `init` | fn(module pointer) -> int | plugin | 0 on success. |
| 5 | 0x14 | `process` | fn(module pointer, pointer to 16-bit samples, int frames, int bits_per_sample, int channels, int sample_rate) -> int | plugin | Process **in place**; return the frame count to output (between **half and double** the input). |
| 6 | 0x18 | `quit` | fn(module pointer) | plugin | |
| 7 | 0x1C | `user_data` | pointer | plugin | Free for your use. |

### 8.3 Call order

1. At startup (and when the selection changes) the host loads the DLL named in
   `config_dspplugin_name`, resolves `winampDSPGetHeader2`, calls it, calls
   `get_module(config_dspplugin_num)`, sets the parent-window and instance
   members, and calls `init`. A null header or module unloads the DLL
   (S09 §6.3).
2. **Only one DSP module is active at a time** (one stored file name and index).
3. Input plugins reach you through `dsp_is_active` and `dsp_process`.
   Processing is **synchronous on the input plugin's decode thread** (S09 §6.3).
4. `quit` is called when the DSP is deselected and at exit; **the DLL is freed
   right after** (S09 §6.3).
5. Because the buffer is processed **in place**, the input plugin must provide
   room for up to **twice** the frames it passes when `dsp_is_active` is
   nonzero (S09 §6.3). That is a rule the *host side* follows; your side of it is
   that you may return up to double the input frame count.

### 8.4 Threading

`process` runs on the input plugin's decode thread (S09 §6.3). `init`, `quit`
and `config` follow the kind-wide config/About rule: `config` is called on the
UI thread when the user presses Configure. Which thread calls `init` and `quit`
in 2.95 is **[open: U-2]** for the vis case and unstated for DSP.

### 8.5 Ownership and lifetimes

- The header and module are yours, static, stable; the host writes
  `parent_window` and `dll_instance` in place.
- The sample buffer in `process` is the **caller's**, in place; write the result
  back into it and do not allocate a replacement the caller cannot see. S09 does
  not state the buffer's capacity beyond "room for up to twice the frames"
  **[provisional]**: stay within `2 x frames x channels x (bits/8)` bytes.
- `user_data` is yours to allocate and free.
- **Your DLL is freed after `quit`** (S09 §6.3) — the same strictness as the vis
  kind.
- Which callbacks may be null is not settled **[open]**. The crate models all as
  `Option`; implement `init`, `process` and `quit` at minimum, and note that
  `process` returning between half and double is what the host's buffer sizing
  assumes.

### 8.6 Units

| Quantity | Unit |
|---|---|
| `process` samples | 16-bit signed samples (S09 §6.2 names the pointer "16-bit samples"); interleaved |
| `process` frames | sample frames; block sizes of **at least 128 frames are expected, but not guaranteed** |
| `process` bits per sample, channels, sample rate | bits, count, Hz |
| `process` return | output frame count, between half and double the input |
| `dsp_process` (host side) input | at least 128 frames expected |

**The order in which `dsp_process` applies the DSP plugin and the equalizer is
`[open: U-12]`** — do not assume your plugin sees pre-EQ or post-EQ samples.

### 8.7 A minimal DSP plugin, in prose

1. Put one 12-byte `winampDSPHeader` in static data with `version = 0x20`, a
   persistent `description`, and `get_module` returning your `index`-th module
   or null.
2. Put one `DspModule` in static data: `description`, `config`, `init`,
   `process`, `quit`, `user_data`.
3. Export `winampDSPGetHeader2` as an undecorated C function taking no arguments
   (for a 2.x-targeted build) and returning the header pointer.
4. In `init(module)`: set up your state through `user_data` and return 0.
5. In `process(module, samples, frames, bits, channels, sample_rate)`: transform
   the interleaved samples in place and return the number of frames now in the
   buffer, between half and double the input count. Expect blocks of at least
   128 frames but do not rely on it.
6. In `quit(module)`: free your state. The DLL is freed right after this
   returns.
7. In `config(module)`: open your dialog; it is called from the UI thread.
8. Do not start threads of your own: processing is synchronous on the input
   plugin's decode thread, and your DLL is gone after `quit`.

---

## 9. General purpose plugins (`gen_`)

A general purpose plugin is a service or an extra window that hooks into the
player: it typically subclasses the main window procedure, installs timers, adds
menu items via IPC, or creates a window (embedded via the S11 embed API)
(S09 §7.2).

### 9.1 Entry point

| | |
|---|---|
| Export | `winampGetGeneralPurposePlugin` |
| Arguments | none |
| Returns | pointer to your static `winampGeneralPurposePlugin` |
| Version | `0x10` (`0x11` is 5.x-only, wide description) |
| Size | 28 bytes |

### 9.2 `winampGeneralPurposePlugin`, field by field

| # | Offset | Member | Type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | `version` | int | plugin | `0x10` in 2.95. |
| 1 | 0x04 | `description` | pointer to ANSI string | plugin | |
| 2 | 0x08 | `init` | fn() -> int | plugin | Called after the host fills parent window and instance handle. **0 = success.** |
| 3 | 0x0C | `config` | fn() | plugin | Open configuration (**no arguments**). |
| 4 | 0x10 | `quit` | fn() | plugin | Called at exit. |
| 5 | 0x14 | `parent_window` | window handle | host | Winamp main window. |
| 6 | 0x18 | `dll_instance` | module handle | host | |

Note the field order: `init`, `config`, `quit` come **before** the two
host-filled members, and `config` takes **no** parent-window argument — unlike
every other kind. Use `parent_window` from the struct instead.

The role of `init`'s return value in 2.95 is **[open: U-8]**. In 5.66 a nonzero
return unloads the plugin and marks it NOT LOADED (5.x-only). Return 0.

### 9.3 Call order

1. The host scans `GEN_*.DLL`, loads each, resolves
   `winampGetGeneralPurposePlugin`, calls it, checks the version, sets
   `parent_window` and `dll_instance`, then calls `init` (S09 §7.2).
2. 5.x preloads a fixed list of its own general-purpose plugins first
   (including the media library); the 2.95 full installer also ships the media
   library plugin. **2.95 ordering is not known [open]** (S09 §7.2).
3. A typical plugin subclasses the main window procedure in `init`, installs
   timers, adds menu items via IPC (S10) or creates windows (embedded via S11)
   (S09 §7.2).
4. At exit the host calls `quit` in **reverse load order** and **does not free
   the DLLs** (S09 §7.2).
5. `config` is invoked on the UI thread when the user presses Configure
   (S09 §7.2). Whether it is modal and what it is parented to in 2.95 is
   **[open: U-15]**.

**All calls to general purpose plugins happen on the UI thread**; the plugin may
create its own threads (S09 §7.2).

### 9.4 Ownership and lifetimes

- The struct is yours, static, stable; the host writes `parent_window` and
  `dll_instance` in place.
- **Your DLL is not freed** at shutdown (S09 §1, §7.2) — the same rule as input
  and output, and the reason `quit` must undo any window subclassing you
  installed: a leftover subclass procedure pointing into a DLL that were freed
  would be a crash, and the host deliberately keeps the DLL alive instead.
- Which callbacks may be null is not settled **[open]**. Implement `init`,
  `config` and `quit`.
- If you create threads of your own, S09 §7.2 allows it, and `quit` is where you
  must stop them **[provisional]**.

### 9.5 A minimal general purpose plugin, in prose

1. Put one `winampGeneralPurposePlugin` in static data with `version = 0x10`, a
   persistent `description`, and `init`, `config`, `quit`.
2. Export `winampGetGeneralPurposePlugin` as an undecorated C function taking no
   arguments and returning the pointer to that struct.
3. In `init`: return 0 on success. Do your hooking here — subclass the main
   window procedure, install timers, add menu items through IPC, or create your
   window for the embed API.
4. In `config`: open your configuration dialog. There is no argument; take the
   parent window from the struct's `parent_window`.
5. In `quit`: undo everything `init` did and free your resources. Your DLL is
   not freed, but nothing else is promised.
6. Remember that every call arrives on the UI thread; if you need background
   work, that thread is yours to create and yours to stop in `quit`.

---

## 10. Units at the boundary

Collected from S09 §9 and the per-kind sections above.

| Item | Fact |
|---|---|
| PCM handed to the analyzer/vis feed calls and to `dsp_process` | **Interleaved**; little-endian **signed** integers for 16-bit and wider; **unsigned bytes** for 8-bit |
| Bit depths seen in 5.x | 8, 16, 24, 32; in 2.95, 8 and 16 are expected **[open: U-10]** |
| Visualization row length | **576 bytes** per spectrum row and per waveform row |
| Visualization rows | at most **2** per kind; row 0 = left, row 1 = right |
| Classic analyzer row (advanced path) | 75 bytes (spectrum), 75 bytes (oscilloscope); encoding **[open: U-17]**, belongs with S08 |
| Feed-call call rate | at least **576 sample frames per call** for `analyzer_feed_pcm` and `vis_feed_pcm` |
| Output `write` chunk | at most **8192 bytes** |
| `dsp_process` input | at least **128 frames** expected |
| Timestamps | milliseconds; the input plugin derives them from the output's **written time** |
| Volume | 0..255 everywhere |
| Pan | input side **-127..127**, output side **-128..128** |
| EQ bands | 0..63, 31 = 0 dB, 0 = +12 dB, 63 = -12 dB |
| `set_info` sample rate | **kHz** |
| `vis_set_format` / `VisModule::sample_rate` | **Hz** |
| `get_file_info` length | ms, `-1` = unknown or stream |
| Vis `latency_ms` / `delay_ms` | ms; `latency_ms` shifts the frame you get, `delay_ms` is the host's sleep after each render |

**The 576-sample vis frame.** A visualization frame the host stores in its ring
is `(spectrum_rows + waveform_rows) x 576` bytes plus a 4-byte timestamp, and
the module's two arrays are each `2 x 576` bytes (S09 §5.2, §5.3). The host
copies `spectrum_rows` rows from `spectrum_data` and then `waveform_rows` rows
from `waveform_data` into the frame, so the arrays you read in `render` are
always at least that large and always fully written: when no frame matches, or
nothing is playing, **both arrays are zeroed completely** (S09 §5.3). The
matching tolerance (how close a frame's timestamp must be) is a 5.x number —
the classic analyzer accepts a match within 400 ms, plugins use a ~200 ms early
stop, a single-frame ring always returns its frame (S09 §3.8) — and the 2.95
numbers are **[open: U-11]**.

---

## 11. What the host guarantees, and what it does not

### 11.1 Guaranteed (S09)

- **Calling convention and discovery.** cdecl callbacks; undecorated export
  names; the host looks for the export of the kind it is scanning for.
- **Field order, size and offsets** as tabulated, with no padding, on 32-bit
  x86.
- **The struct stays put.** Your static struct keeps its address for the DLL's
  life; the host writes its members in place.
- **Host-filled members are filled before you are called.** Input: `main_window`
  and `dll_instance` and the eleven host-callback slots before `init`, with
  `set_info` installed only after `init` returns. Output: `main_window` and
  `dll_instance` before `init`. Vis: `parent_window`, `dll_instance`,
  `sample_rate` and `channels` before `init` (and `parent_window` valid before
  `config`). DSP: `parent_window` and `dll_instance` before `init`. Gen:
  `parent_window` and `dll_instance` before `init`.
- **`out_module` is set before `play`** when your input plugin's flag bit 0 is
  set.
- **The input call order** of §5.3, including the `analyzer_init` →
  `vis_set_format` → `set_info` → decode-thread sequence, the end-of-track
  notification, and `analyzer_deinit` in `stop`.
- **The vis cadence of §7.3**, including the dedicated vis thread, the
  zeroed-arrays rule, the extra render after playback stops, the `delay_ms`
  sleep, the paused behaviour, and `quit` on the vis thread before the DLL is
  freed.
- **One visualization active at a time.**
- **The output `write`/`can_write`/`is_playing` contract** the input plugin's
  loop is written against: non-blocking write of at most 8192 bytes, `can_write`
  that does not shrink except through `write` or `close`, and `is_playing`
  going to 0 only when the buffered audio has been heard.
- **Unload behaviour:** input, output and gen DLLs are not freed; vis and DSP
  DLLs are freed after `quit`.
- **The classic host surface exists:** a main window whose class name is
  `Winamp v1.x`, the `WM_WA_IPC` channel, `WM_WA_MPEG_EOF`, and the other
  window classes of S09 §8. Under Rustamp this is a fake window presented by the
  32-bit plugin host (plugin-authors.md §2), and the *details* of the emulated
  surface are the not-yet-written S10/S11.

### 11.2 Not guaranteed

Everything below is explicitly unsettled. Do not build on it; if you must
choose, choose the conservative reading.

| Question | ID |
|---|---|
| Whether posting `0x0402` ends the track and advances the playlist in 2.95, and what other private `WM_USER+n` messages the host accepts | U-1 |
| Whether 2.95 runs visualizations on a separate thread, and which thread calls `config`/`init`/`quit` | U-2 |
| Waveform byte sign convention and offset, and spectrum scaling, in 2.95 | U-3 |
| Whether 2.95 overwrites the output `id` or compares ids directly | U-4 |
| Visualization discovery in 2.95 (prefix filter or any DLL exporting the entry point) | U-5 |
| Return value for unknown IPC numbers in 2.95 | U-6 |
| Version acceptance per kind in 2.95: strict equality or a range | U-7 |
| Role of the general purpose `init` return value in 2.95 | U-8 |
| Whether 2.95 passes any argument to `winampDSPGetHeader2` and `winampVisGetHeader` | U-9 |
| Bit depths accepted by the two PCM feed calls in 2.95 (8 and 16 expected) | U-10 |
| Ring sizing and time-matching tolerances in 2.95 | U-11 |
| Order in which `dsp_process` applies the DSP plugin and the equalizer | U-12 |
| Exact signatures of the optional extended-info exports in 2.95 | U-13 |
| Whether the 2.95 host filters `file_extensions` list elements by length and how it treats empty pairs | U-14 |
| Calling context of the general purpose `config` in 2.95 (modal? parent?) | U-15 |
| Which input-module callbacks the 2.95 host tolerates as null | U-16 |
| Encoding and value range of the 75-byte analyzer rows | U-17 |
| How PCM is sliced into spectrum windows | U-18 |
| Whether the `file_extensions` pointer may be null in static data | U-19 |

Additional unsettled points that are ours rather than S09's, from
`crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` (the crate's own numbering):

| Question | OQ |
|---|---|
| Output `id` is typed `c_int` although a 5.x host stores a pointer in it; a 5.x-aware host on x86 must reinterpret it | OQ-1 |
| The `write` data pointer's constness/signedness is unspecified | OQ-2 |
| The element types of the PCM/frame/row pointers are not given (`*mut c_void`) | OQ-3 |
| The `eq_set` band pointer is `*mut c_char` by convention only | OQ-4 |
| String constness of `get_file_info`/`info_box`/`is_our_file`/`play` and of all `description` members is unspecified | OQ-5 |
| `file_extensions` nullability (same as U-19) | OQ-6 |
| The 39th input member's type is modelled as `*mut c_void` in a separate `InModuleExt` | OQ-7 |
| **Which DSP header size a host should expect for a `0x20` header** (12 or 16) | OQ-8 |
| Vis waveform bytes are `u8` because the 2.95 sign is U-3 | OQ-9 |
| `get_module` returns are typed as pointers to the respective module structs; S09 says only "module pointer" | OQ-10 |
| **Entry-point function types are not defined by S09**; our convention is `extern "C" fn() -> *mut T` | OQ-11 |
| "No padding" is asserted by S09 and relied on; the crate's asserts check it on x86 only | OQ-12 |
| The optional extended-info exports have names but no signatures (same as U-13) | OQ-13 |
| **Vis `config` call order**: `parent_window` validity before `config` is documented-but-unverified | OQ-14 |

### 11.3 5.x-only, so do not use it in a 2.95-targeted plugin

From S09 §10, restricted to the five kinds this guide covers:

- **Input:** the wide-character exports (`winampGetExtendedFileInfoW`,
  `winampSetExtendedFileInfoW`, `winampGetExtendedRead_openW` and relatives),
  `winampUseUnifiedFileInfoDlg`, `winampAddUnifiedFileInfoPane`, version flags
  `0x0F000000` and `0xF0000000` and the 39th member, version `0x101`.
- **Output:** `winampGetOutModeChange`, version `0x11`.
- **Visualization:** header `0x102` with the main-window-taking entry point, the
  vis command window (IPC 611) and command ids 40382..40391.
- **DSP:** header `0x21`/`0x22` with the main-window-taking entry point and
  `key_check`.
- **General purpose:** version `0x11` and the `init`-status semantics.
- **All kinds:** `winampUninstallPlugin`, plugin safe mode and its
  description-string naming convention.

### 11.4 Not the mediahost path

RFC 0006 plans separate GPL helper processes (`rustamp-dechost`,
`rustamp-dsphost`) for *compiled-in* decoder and DSP code that cannot live in
the player's process for licensing or stability reasons. **That is not how your
classic DLL is hosted.** RFC 0006 §2 puts bridging classic 32-bit `dsp_` DLLs out
of `plughost32` explicitly out of scope, and RFC 0006 §6 keeps classic `dsp_`
DLLs in `plughost32`; the mediahost residents are a different, later mechanism
for code that is linked into a helper rather than loaded as a plugin
(plugin-authors.md §7).

---

## 12. Open questions this guide carries

Everything in §11.2 is unresolved. In addition, the following are gaps in this
guide's own coverage, listed so they are not mistaken for facts:

1. **Where Rustamp looks for plugin DLLs.** S09 states the classic location
   (`Plugins` under the Winamp folder, `PluginDir`), and plugin-authors.md says
   plugins are user-installed, but neither states the directory Rustamp scans
   or how it is configured. **[open]**
2. **How the classic IPC surface is emulated** (`WM_WA_IPC` numbers, the INI
   and plugin-directory queries, `WM_COMMAND` ids). S10 is not written.
   **[open]**
3. **The embed API** by which a `vis_` or `gen_` window is hosted in a Winamp
   frame (`Winamp Gen`, `WinampVis` geometry). S11 is not written. **[open]**
4. **The numerical processing of the 576-entry arrays and the analyzer rows.**
   S08 is not written; S09 defers to it (and U-17, U-18 remain open).
   **[open]**
5. **The compatibility matrix.** Which real plugins Rustamp has been verified to
   run is tracked as a test matrix, not as a per-plugin guarantee
   (plugin-authors.md §3), and no list is published. **[open]**
6. **The host's current state.** Per the handoff, `rustamp-plughost` is still a
   stub and the plugin host is milestone M3 work, so nothing in this guide has
   been exercised against a running Rustamp host. The ABI types are pinned and
   asserted; the host that calls through them is not built. **[open]**

---

## 13. Where to read more

| Document | What it is |
|---|---|
| [docs/notes/plugin-authors.md](notes/plugin-authors.md) | Licensing: under what terms your plugin runs, and what Rustamp asks of you |
| [specs/S09-plugin-abi.md](../specs/S09-plugin-abi.md) | The audited ABI spec behind every fact in this guide, with its provenance tags and its open items |
| `crates/rustamp-plugin-abi/src/` | The concrete types, constants and export names, with the layout asserts |
| `crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` | What the crate's modelling leaves unsettled |
| [docs/rfc/0004-plugin-licensing.md](rfc/0004-plugin-licensing.md) | The accepted licence split |
| [docs/rfc/0006-out-of-process-hosting.md](rfc/0006-out-of-process-hosting.md) | Decoder/DSP helper processes (not the classic plugin path) |
| [docs/HANDOFF.md](HANDOFF.md) | Project status and the open items list |
| [CLEANROOM.md](../CLEANROOM.md) | The binding rules on sources for everyone working on Rustamp |
