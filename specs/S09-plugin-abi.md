---
Spec ID: S09
Title: Winamp 2.x plugin ABI (input, output, visualization, DSP, general purpose)
Sources consulted (category only): Winamp 2.95 binaries (static inspection only); Winamp SDK headers; Winamp 5.x source (host-side plugin loading and visualization data path); free: BSD-licensed visualization plugin source (consumer-side behavior only)
Author agent: dirty-room research agents (source pass and static binary pass)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14
Winamp version validated against: 2.95 (static inspection of the installer payload only; no binary was executed)
Status: audited – static; runtime confirmation pending
---

# S09 - Winamp 2.x plugin ABI

This document contains facts only: layouts, offsets, constants, call order and behavior in prose. Member names used below are neutral descriptive names chosen for this document. They are not part of the ABI; only the order, size and offset of each member matter. Names that plugins and tools depend on (exported entry-point names, the five module/header type names, window class names, message numbers) are given in their public form.

## 0. How to read this document

Every statement carries a provenance tag so implementers know how far to trust it.

| Tag | Meaning |
|---|---|
| **[295]** | Confirmed by static inspection of the Winamp 2.95 binaries (plugin static data, export tables, host string table, host import list). |
| **[SDK]** | Taken from the public plugin development headers, which describe the contract. Believed to hold for 2.95 unless marked 5.x. |
| **[5X]** | Observed in the Winamp 5.x host. 2.95 most likely behaves the same, but this is NOT confirmed. Needs runtime verification. |
| **[5X-ONLY]** | A feature that exists only in 5.x. A 2.95-level host may skip it, but plugins from the 5.x era may use it. |
| **[UNV]** | Unverified. Collected again in section 12. |

Scope: layouts are for a 32-bit x86 Windows process (all 2.95 plugins are 32-bit x86 [295]). Integers are 32-bit unless stated. Pointers, window handles and module handles are 32 bits wide, and every function-pointer member has the same width. "Host-filled" means the host writes the member after loading the DLL. "Plugin-filled" means the plugin initialises it statically or at run time. Strings are NUL-terminated single-byte strings in the system ANSI code page [SDK]; wide-string variants are [5X-ONLY]. Members are 4-byte aligned and there is no padding anywhere in the structures below (the only arrays are byte arrays whose lengths are multiples of 4).

Consequence for Rustamp: these structures hold raw 32-bit code pointers and use the cdecl convention. A host can load such DLLs in-process only if it is itself a 32-bit x86 process. A 64-bit or non-Windows build must use a 32-bit helper process or offer a different (native) plugin API. That decision is outside this spec.

Notes for a Rust implementer (facts, not code): every structure is `#[repr(C)]`; on 32-bit x86 Windows the plain C calling convention is cdecl, so callbacks are `extern "C"`; callback members may be null unless stated otherwise, so model them as optional; the structures live in the plugin's static data and must stay valid and at a stable address for as long as the DLL is loaded, and the host writes into several of them in place.

## 1. Conventions common to all plugin kinds

| Topic | Fact | Tag |
|---|---|---|
| Calling convention | Every callback inside the structures and every entry-point export uses cdecl (caller cleans the stack). Exports are undecorated C names (no leading underscore, no @n suffix). | [SDK] [295] |
| Entry points | Each plugin DLL exports one "get header or module" function whose name identifies its kind (section 2). The host finds it by name lookup in the loaded module and looks only for the names of the kind it is scanning for. | [295] [SDK] |
| Member order | The order in the tables below is the ABI. | [SDK] [295] |
| Version field | The first member of every header/module is an integer version. The host refuses a plugin whose version it does not accept. | [SDK] [5X] |
| Parent window | The host-filled main-window member is the handle of Winamp's main window, whose window class name is `Winamp v1.x` (kept for compatibility). Plugins use it to send IPC messages, to parent dialogs and (general purpose plugins) to subclass it. | [SDK] [295] |
| DLL instance | The host-filled instance member is the plugin's own module handle, used for resource lookup. | [SDK] |
| Config / About | Called when the user presses the corresponding button on the Plug-ins page of Preferences, on the UI thread. Argument is the parent window handle or the module pointer depending on kind (see per-kind tables). | [SDK] [5X] |
| Where plugins live | A `Plugins` sub-folder of the Winamp folder. 2.95 keeps its name in a setting called `PluginDir`; the string `Plugins` is present in the host. | [295] |
| Unload behavior | Input, output and general purpose DLLs are not freed at shutdown (plugins may have subclassed windows). Visualization and DSP DLLs are freed after their Quit. | [5X] |
| Exception isolation | The 5.x host wraps visualization Init/Render/Quit (and some other quit calls) in structured exception handlers, switchable by a hidden setting, and shows a numbered plugin-error dialog on a fault. Unknown for 2.95. | [5X] [UNV] |
| Safe mode | Plugins are matched against a built-in allowlist using a description-string naming convention. Not needed for 2.95. | [5X-ONLY] |
| Uninstall hook | Optional export `winampUninstallPlugin` taking (instance handle, preferences window handle, int) and returning an "uninstall now" / "uninstall after restart" code. Return values differ per kind: input and general purpose 1 = now, 0 = restart; DSP 0 = now, 1 = restart; visualization 0 = now (always without restart); output only supports restart (0). Not present in 2.95. | [5X-ONLY] |
| Plugin ordering | In 5.x the input plugins are loaded in a fixed preferred-name order (the MP3 plugin first, plugins with very large extension lists last), then the rest in file enumeration order. 2.95 ordering is not known. | [5X] [UNV] |

## 2. Discovery and entry points

| Kind | File-name pattern scanned [295 strings] | Exported entry point | Arguments | Returns | Version expected |
|---|---|---|---|---|---|
| Input | `IN_*.DLL` in the Plugins folder | `winampGetInModule2` | none | pointer to the plugin's static `In_Module` | 0x100 |
| Output | `OUT_*.DLL` | `winampGetOutModule` | none | pointer to the plugin's static `Out_Module` | 0x10 |
| Visualization | Any `*.DLL` in the Plugins folder may be probed for the export (a `vis_` prefix filter is not seen as a string) [UNV] | `winampVisGetHeader` | none in 2.x (header 0x101). 5.x passes the main window handle (header 0x102); under cdecl an unused extra argument is harmless | pointer to static `winampVisHeader` | 0x101 (0x102 [5X-ONLY]) |
| DSP | `DSP_*.DLL` | `winampDSPGetHeader2` | none in 2.x; main window handle in 5.x (harmless extra argument) | pointer to static `winampDSPHeader` | 0x20 (0x21/0x22 [5X-ONLY]) |
| General purpose | `GEN_*.DLL` | `winampGetGeneralPurposePlugin` | none | pointer to static `winampGeneralPurposePlugin` | 0x10 |

Notes:

- [295] The export names above are present in the 2.95 host's string table. The optional input exports `winampGetExtendedFileInfo` and `winampGetExtendedRead_open` / `_getData` / `_close` are also present there (section 3.9).
- [295] The host string table also holds these setting names: `config_outname` (selected output plugin file name), `config_visplugin_name`, `config_visplugin_num`, `config_visplugin_autoexec`, `config_visplugin_priority`, `config_dspplugin_name`, `config_dspplugin_num`, `config_defext`. S12 covers their semantics.
- [295] The Plug-ins preferences list shows a bracketed "NOT LOADED" label for a plugin that could not be loaded.
- Output plugins: the 5.x host preloads its standard outputs first (DirectSound, waveOut, disk writer) and then the rest [5X]; at most 32 output modules are listed [5X].
- The returned module pointer must stay valid for the life of the DLL (it points at static data) [SDK].

## 3. Input plugin (`In_Module`)

### 3.1 Layout (2.95)

Size in 2.95: 38 members x 4 bytes = 152 bytes (0x98). [295] This was checked in the static initial data of two 2.95 input plugins: version 0x100, description pointer, zero in every host-filled member, code pointers in the plugin callbacks, a code pointer in the EQ slot, and zero in the final member.

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | version | signed 32-bit int | plugin | 0x100 for all 2.95 plugins [295]. |
| 1 | 0x04 | description | pointer to ANSI string | plugin | Human-readable name and version for the Plug-ins list. |
| 2 | 0x08 | main_window | window handle | host | Winamp's main window; set before Init. |
| 3 | 0x0C | dll_instance | module handle | host | Plugin's own module handle. |
| 4 | 0x10 | file_extensions | pointer to double-NUL-terminated string list | plugin | Pairs of strings: extension list, then display name (format in 3.4). May be rewritten by the plugin from its Config (user-selectable extensions). |
| 5 | 0x14 | seekable | int (0/1) | plugin | Whether the current stream is seekable. May change while playing. |
| 6 | 0x18 | flags | int (bit flags) | plugin | Bit 0: audio goes through a Winamp output plugin. 2.95 plugins hold 1 or 0 here [295]. Other bits: 3.3. |
| 7 | 0x1C | config | function (window handle) | plugin | Open the configuration dialog. |
| 8 | 0x20 | about | function (window handle) | plugin | Open the about box. |
| 9 | 0x24 | init | function () | plugin | Called once at startup after the host fields are set. Returns nothing in 2.x; 2.x ignores any return value. |
| 10 | 0x28 | quit | function () | plugin | Called once at exit. |
| 11 | 0x2C | get_file_info | function (ANSI path or null, title buffer, pointer to int length-in-ms) | plugin | Fill title and length for a file. A null path means the file currently playing. The title buffer holds 2048 bytes [SDK]. Length -1 means unknown or stream. |
| 12 | 0x30 | info_box | function (ANSI path, window handle) returning int | plugin | Show the file-info dialog. Returns 0 if the tags were edited, 1 if unchanged. |
| 13 | 0x34 | is_our_file | function (ANSI name) returning int | plugin | Called before extension matching so a plugin can claim non-file names (URLs, special schemes). Nonzero = this plugin handles it. |
| 14 | 0x38 | play | function (ANSI path) returning int | plugin | Start playback. Returns 0 on success, -1 for file not found, any other nonzero for another error that stops the playlist advance. |
| 15 | 0x3C | pause | function () | plugin | Pause. |
| 16 | 0x40 | unpause | function () | plugin | Resume. |
| 17 | 0x44 | is_paused | function () returning int | plugin | 1 if paused, 0 otherwise. |
| 18 | 0x48 | stop | function () | plugin | Stop and unload the stream. The plugin must stop its decode thread and close the output (if used) before returning. |
| 19 | 0x4C | get_length | function () returning int | plugin | Track length in ms. |
| 20 | 0x50 | get_position | function () returning int | plugin | Current playback position in ms; normally the output module's played time. May be called from threads other than the UI thread. |
| 21 | 0x54 | seek | function (int ms) | plugin | Seek. Normally signals the decode thread to seek and then flushes the output at the new time. |
| 22 | 0x58 | set_volume | function (int) | plugin | 0..255; normally forwarded to the output module. |
| 23 | 0x5C | set_pan | function (int) | plugin | -127..127 on the input side (output plugins use -128..128); normally forwarded to the output module. |
| 24 | 0x60 | analyzer_init | function (int max_latency_ms, int sample_rate) | **host** | Plugin calls once in `play`, after the output is open, passing the latency returned by the output `open` (or 0) and the sample rate. Prepares the host's analyzer and visualization buffering. |
| 25 | 0x64 | analyzer_deinit | function () | **host** | Plugin calls from `stop`. |
| 26 | 0x68 | analyzer_feed_pcm | function (pointer to PCM, int channels, int bits_per_sample, int timestamp_ms) | **host** | Easy path for the classic in-window analyzer: hand over interleaved PCM and the host computes spectrum/scope itself. Needs at least 576 sample frames per call [SDK]. |
| 27 | 0x6C | analyzer_get_mode | function () returning int | **host** | Advanced path: which analyzer data the host wants. Bit 0 (1) spectrum, bit 1 (2) oscilloscope, value 4 none/window-shade; 3 = both. |
| 28 | 0x70 | analyzer_feed_rows | function (pointer to data, int timestamp_ms, int mode) returning int | **host** | Advanced path: the plugin supplies precomputed analyzer rows for the given mode value. Nonzero return = host is not collecting data. |
| 29 | 0x74 | vis_feed_pcm | function (pointer to PCM, int channels, int bits_per_sample, int timestamp_ms) | **host** | Easy path for visualization plugins; same arguments as `analyzer_feed_pcm`; at least 576 sample frames per call. |
| 30 | 0x78 | vis_get_mode | function (pointer to int spectrum_rows, pointer to int waveform_rows) returning int | **host** | Advanced path: how many spectrum and waveform rows (0, 1 or 2 each) the running visualization wants. Returns nonzero when a visualization is running; both outputs are 0 otherwise. |
| 31 | 0x7C | vis_feed_frame | function (pointer to data, int timestamp_ms) returning int | **host** | Advanced path: supply one frame laid out as the spectrum rows followed by the waveform rows, 576 bytes per row. Nonzero return = not accepted. |
| 32 | 0x80 | vis_set_format | function (int sample_rate, int channels) | **host** | Called once in `play` to announce the stream format to the visualization. Argument order is sample rate, then channels (some older third-party headers had them reversed; this is the correct order). |
| 33 | 0x84 | dsp_is_active | function () returning int | **host** | Whether the host will modify samples. 0 = no. Nonzero = yes, and the output frame count may exceed the input count, so the plugin must size its buffer for up to twice the input. |
| 34 | 0x88 | dsp_process | function (pointer to 16-bit signed samples, int frames, int bits_per_sample, int channels, int sample_rate) returning int | **host** | Runs the active DSP plugin and EQ over the block in place. Returns the number of frames now in the buffer (up to double the input). |
| 35 | 0x8C | eq_set | function (int on, pointer to 10 band bytes, int preamp) | plugin | Optional. A plugin with its own EQ implements it; the host calls it when EQ settings change. Values 0..63: 31 = 0 dB, 0 = +12 dB, 63 = -12 dB. A plugin without its own EQ leaves a stub that ignores calls [295]. |
| 36 | 0x90 | set_info | function (int bitrate_kbps, int sample_rate_khz, int channels, int synched) | **host** | Plugin reports stream properties for the main-window readouts. -1 means "no change" per argument. Sample rate is in kHz (e.g. 44), channels is the count, `synched` is a 0/1 flag. |
| 37 | 0x94 | out_module | pointer to `Out_Module` | **host** | Selected output plugin; set by the host before `play` when flag bit 0 is set. |

Total 0x98 bytes. [295] The host fills members 2, 3, 24-34, 36 and 37 [295: all read as zero in the static data of 2.95 plugins; member 35 holds a code pointer].

5.x-only tail: a 39th member (pointer to the host service table, offset 0x98) exists only if the version value carries the "init returns a status" flag (3.2). [5X-ONLY]

### 3.2 Version constants

| Value | Meaning | Tag |
|---|---|---|
| 0x100 | Winamp 2.x input module. The only value 2.95 plugins use. | [295] |
| 0x101 | 5.64+ variant allowing a wide-character description. | [5X-ONLY] |
| 0x0F000000 (OR-ed into version) | Plugin uses wide-character file names. A 5.x build of one third-party module uses 0x0F000100; its 2.x-targeted build uses plain 0x100. | [5X-ONLY]; plain value [295] |
| 0xF0000000 (OR-ed into version) | `init` returns a status (0 success, 1 failure) and the 39th member is filled by the host. | [5X-ONLY] |

Host acceptance test (5.x): clear the two flag groups, then require the remainder to be 0x100 or 0x101 [5X]. A 2.95-level host should accept exactly 0x100 [UNV].

### 3.3 Bits of the flags member (5.x-era meaning)

| Bit value | Meaning | Tag |
|---|---|---|
| 1 | Audio is routed through Winamp's output plugin. The only bit 2.95 knows. | [295] |
| 2 | Plugin does its own equalization; the host does not apply its EQ. | [SDK] [5X-ONLY] |
| 8 | Plugin already applied replay gain. | [5X-ONLY] |
| 16 | Plugin already applied the replay-gain preamp. | [5X-ONLY] |

The 5.x host clears bits 2, 4, 8 and 16 before calling `init`, so a plugin that wants them must set them from within `init` [5X].

### 3.4 `file_extensions` format

A sequence of string pairs ended by an empty string. Each pair is an extension list followed by a display name. Extensions within a list are separated by semicolons and written without dots. The host takes the extension after the last dot of the file name and compares it case-insensitively with each list element; elements about 15 characters or longer are not matched [5X]. Plugins with very many extensions are listed last in the Open File dialog filter because that filter has a 260-character limit [5X]. The buffer can be edited at run time.

If no plugin claims a file through `is_our_file` or by extension, the host falls back to the default-extension setting (`config_defext`, default `mp3` [295 string]) and picks the plugin claiming that extension [5X].

### 3.5 Lifecycle and call order

Host start:

1. Load the DLL, look up `winampGetInModule2`, call it with no arguments, receive the module pointer, verify the version.
2. The host writes the instance handle, main window handle, and the host-provided members: `dsp_process`, `dsp_is_active`, `analyzer_get_mode`, `analyzer_feed_rows`, `analyzer_init`, `analyzer_deinit`, `vis_set_format`, `vis_feed_frame`, `vis_get_mode`, `analyzer_feed_pcm`, `vis_feed_pcm`. It clears `set_info` and sets it to its own function after `init` returns.
3. Call `init()`, then add the module to the list.

Per play request (main thread):

1. Choose a module: a first pass calls every module's `is_our_file` in list order; a second pass matches extension lists in list order.
2. If flag bit 0 is set, pick the output named by `config_outname`; if unavailable fall back to the DirectSound output, then waveOut; if none exists, show an error and abort. The host sets `out_module` (and the output's main window), then applies volume and pan.
3. Call `play(path)`. The plugin opens the file, calls the output `open`, then `analyzer_init(latency, rate)`, `vis_set_format(rate, channels)` and `set_info(...)`, and starts its decode thread.
4. Decode thread loop: decode, optionally `dsp_is_active` / `dsp_process`, then `vis_feed_pcm` / `analyzer_feed_pcm`, then output `can_write` / `write`. Timestamps come from the output's written-time.
5. End of file: the plugin waits for the output's `is_playing` to go false, then posts the end-of-track notification (3.6).
6. User stop: the host calls `stop()`; the plugin stops its decode thread, closes the output and calls `analyzer_deinit()`.

Host exit (main thread): call `quit()` on every loaded input module in reverse load order. The DLL is not freed.

[295] confirmed: the layout and the presence of the callback slots. The ordering above is [5X] and matches the documented usage [SDK].

### 3.6 Threads, and the end-of-track message

| Call | Thread | Tag |
|---|---|---|
| init, quit, config, about, info_box, is_our_file | UI thread | [5X] |
| play, stop, pause, unpause, seek, set_volume, set_pan, eq_set | UI thread; must not block for long | [5X] |
| get_position, get_length, is_paused | UI thread, plus the host's classic-analyzer thread and the visualization thread (the vis loop polls the position every frame); must be thread-safe | [5X] |
| get_file_info | UI thread, possibly worker threads when playlists are built | [UNV] |
| Output module write / can_write / is_playing / open / close | the plugin's decode thread; flush from the seek path | [SDK] |
| Analyzer and vis feed calls, dsp_process | the plugin's decode thread | [SDK] |

End-of-track notification: when a track ends on its own, the input plugin's decode thread POSTS (not sends) window message number 0x0402 (WM_USER + 2, public name `WM_WA_MPEG_EOF`) to the main window with both parameters zero. The plugin posts it only if it was not asked to stop. On receipt the host advances to the next playlist item or stops. [295: every 2.95 input DLL examined contains the immediate value 0x402 and imports a posting API, consistent with this; the receiving behavior in 2.95 is [UNV].] This is not the IPC channel: IPC uses message number 0x0400 with the command in the second parameter, while this notification uses message number 0x0402 itself, so a host must handle it as a separate message.

### 3.7 Output-plugin rules for input plugins

- When flag bit 0 is set the host sets `out_module` before `play`, and the plugin must use that module for audio. If the user changes the output while playing, the change applies on the next play [5X].
- The input plugin calls the output `open` in `play` with the stream's rate, channel count, bit depth and buffer/prebuffer hints (0 = default). The return value is the maximum latency in ms to pass to `analyzer_init`.
- Timing comes from the output: played-time for what is being heard, written-time for the timestamp arguments of the analyzer/vis feed calls.
- On seeking the plugin calls the output `flush(time_ms)`.
- Output `pause` takes an int and returns the previous pause state.
- Without an output plugin (flag bit 0 clear) the input plugin supplies its own volume/pan handling and timekeeping.

### 3.8 Visualization and analyzer data path

This is what feeds both the in-window analyzer and visualization plugins. S08 gives the numerical processing of the 576-entry arrays.

| Mechanism | Host function | What the plugin provides | Notes |
|---|---|---|---|
| Classic analyzer, easy path | `analyzer_feed_pcm` | interleaved PCM, channel count, bit depth, timestamp | The host picks spectrum or scope according to the user's analyzer mode and consumes 576 sample frames per call. Bit depths seen in 5.x: 8, 16, 24, 32; in 2.95: 8 and 16 expected [UNV]. |
| Classic analyzer, advanced path | `analyzer_get_mode` + `analyzer_feed_rows` | a 75-byte spectrum row and/or a 75-byte oscilloscope row | The mode argument says which rows the buffer holds (1 spectrum, 2 scope, 3 both with spectrum first). The row encoding is deferred to S08 [UNV here]. |
| Visualization, easy path | `vis_feed_pcm` | as `analyzer_feed_pcm` | The host builds spectrum and waveform rows for as many channels as the running vis asked for. |
| Visualization, advanced path | `vis_get_mode` + `vis_feed_frame` | one frame: all spectrum rows then all waveform rows, 576 bytes each | Row counts come from `vis_get_mode`. |
| Format announcement | `vis_set_format` | sample rate, channel count | Call before audio flows; the host copies them into the running vis module's rate/channel members. |
| Buffering setup | `analyzer_init(max_latency_ms, sample_rate)` | latency returned by output `open` | The host sizes a ring of timestamped frames. 5.x frame count: integer-rounded max_latency_ms x 4 x sample_rate / 450000, minimum 1. |

Frame selection [5X]: each stored frame carries the timestamp the plugin supplied. A consumer asks for the frame whose timestamp is closest to a target time: the current output position for the classic analyzer, and the output position plus the vis module's `latency_ms` for a visualization plugin. The search starts at the previous result and stops early once the distance is small and growing (about 6 ms for the classic analyzer, 200 ms for plugins). The classic analyzer accepts a match only within 400 ms; otherwise it draws a synthetic idle pattern. A ring with a single frame always returns that frame. A visualization with no frame available gets zeroed arrays.

DSP hooks: `dsp_is_active` returns 0 when nothing in the host will touch the samples; 1 when a DSP plugin is running (or replay-gain correction is needed [5X-ONLY]); 2 when only the host equalizer is active [5X]. `dsp_process` handles interleaved PCM in place and returns the new frame count, between half and double of the input [SDK].

### 3.9 Optional exports of input plugins present in 2.95 [295]

| Export | Present in these 2.95 DLLs | Purpose | Signature (convention from the contract; exact 2.95 signature [UNV]) |
|---|---|---|---|
| `winampGetExtendedFileInfo` | mp3, vorbis, nsv, cdda, dshow plugins | Query a named metadata field of a file ("length", "title", "artist", "type", ...). | (ANSI file, ANSI field name, output buffer, buffer size) returns int; nonzero = field supported |
| `winampSetExtendedFileInfo` | mp3, vorbis, nsv | Stage a metadata edit. | (ANSI file, ANSI field name, ANSI value) returns int |
| `winampWriteExtendedFileInfo` | mp3, vorbis, nsv | Commit staged edits. | () returns int |
| `winampGetExtendedRead_open` / `_getData` / `_close` | cdda plugin | Decode a file to PCM without playing (converter/ripper use). | open(file, out size, out bits, out channels, out rate) returns handle; getData(handle, buffer, length, pointer to cancel flag) returns bytes; close(handle) |
| `CreateAudioDecoder` (mp3 plugin), `CreateVideoDecoder` (NSV decoder helper DLL) | as named | Decoder factories used by other Nullsoft components. | Not part of the general plugin ABI; ignore. |

Extended-file-info lookups are also reachable through host IPC calls (S10).

## 4. Output plugin (`Out_Module`)

### 4.1 Layout (2.95)

Size 20 members x 4 = 80 bytes (0x50). [295] Checked in the static data of two 2.95 output plugins: version 0x10, description pointer, a large constant id, zero for the two host-filled members, then eleven or more code pointers.

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | version | int | plugin | 0x10 in 2.95 [295]. 5.64+ also accepts 0x11 (wide description) [5X-ONLY]. |
| 1 | 0x04 | description | pointer to ANSI string | plugin | Name for the output list. |
| 2 | 0x08 | id | 32-bit int | plugin | Module identifier. 2.95 plugins carry large fixed constants [295]. Third-party modules should use values >= 65536 [SDK]. The 5.x host overwrites this member with a pointer to a copy of the DLL file name and uses it to match the saved selection [5X-ONLY]. |
| 3 | 0x0C | main_window | window handle | host | |
| 4 | 0x10 | dll_instance | module handle | host | |
| 5 | 0x14 | config | function (window handle) | plugin | |
| 6 | 0x18 | about | function (window handle) | plugin | |
| 7 | 0x1C | init | function () | plugin | Called when the output list is built at startup. |
| 8 | 0x20 | quit | function () | plugin | Called at exit. |
| 9 | 0x24 | open | function (int sample_rate, int channels, int bits_per_sample, int buffer_ms, int prebuffer_ms) returning int | plugin | Open the device. Returns the maximum latency in ms (0 for disk writers) or a negative value on failure. Buffer arguments are hints (0 = default, prebuffer <= buffer) that most plugins ignore. A buffer_ms of -666 means the output clock will be used to sync video; the DirectSound output then disables silence trimming. |
| 10 | 0x28 | close | function () | plugin | Close the device. |
| 11 | 0x2C | write | function (pointer to bytes, int length) returning int | plugin | Non-blocking. Length is at most 8192 bytes per call [SDK]. Returns 0 when the data was taken, nonzero when it could not be taken yet. |
| 12 | 0x30 | can_write | function () returning int | plugin | Bytes that can be written right now. Does not shrink except through `write` or `close`. |
| 13 | 0x34 | is_playing | function () returning int | plugin | Nonzero while buffered audio is still audible. The input plugin must wait for 0 before `close` or the end of the song is cut off. |
| 14 | 0x38 | pause | function (int state) returning int | plugin | 1 pauses, 0 resumes; returns the previous state. |
| 15 | 0x3C | set_volume | function (int) | plugin | 0..255. |
| 16 | 0x40 | set_pan | function (int) | plugin | -128..128. |
| 17 | 0x44 | flush | function (int time_ms) | plugin | Drop buffered audio and restart the clock at the given time (seeking). |
| 18 | 0x48 | get_played_ms | function () returning int | plugin | Milliseconds actually played, counted from the last flush time (or 0). |
| 19 | 0x4C | get_written_ms | function () returning int | plugin | Milliseconds written so far (used as visualization timestamps). |

### 4.2 Lifecycle

1. The host loads each `OUT_*.DLL`, calls `winampGetOutModule`, checks the version, writes the instance and main-window members (in 5.x also replaces `id`), and calls `init`. [5X]
2. The user picks the active output in Preferences; the choice is stored by plugin file name in `config_outname` [295 string present].
3. Per playback the input plugin calls `open`, then `write` / `can_write` in a loop from its own thread, then waits on `is_playing`, then `close`. `pause`, `set_volume`, `set_pan` and `flush` come from the UI or seek path through the input plugin.
4. At exit the host calls `quit` on all outputs and does not free the DLLs [5X].
5. [5X-ONLY] Optional export `winampGetOutModeChange(int mode)` tells a plugin when it becomes active or inactive: 0 = no longer active, 1 = becoming active, 0x100 OR-ed in when the change is triggered by playback start. Not in 2.95.

Threading: `open`, `close`, `write`, `can_write`, `is_playing` and `get_written_ms` are called from the input plugin's decode thread; `get_played_ms` may also come from the UI timer thread and the visualization thread; `pause`, `set_volume`, `set_pan`, `flush` from the UI or seek path. Implementations must be thread-safe [SDK] [5X].

2.95 stock outputs: waveOut, DirectSound, disk writer (files out_wave, out_ds, out_disk; the full installer also has out_wm) [295].

## 5. Visualization plugin (`winampVisHeader` / `winampVisModule`)

### 5.1 Header layout (12 bytes)

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | version | int | plugin | 0x101 in 2.x [295: all four 2.95-era visualization libraries examined use it]. 0x102 goes with the main-window-taking entry point [5X-ONLY]. |
| 1 | 0x04 | description | pointer to ANSI string | plugin | Library name. |
| 2 | 0x08 | get_module | function (int index) returning module pointer | plugin | Returns module `index` (0-based) or null if out of range. The host calls 0, 1, 2... until null to list the modules; Preferences stores the library file name and the module index. |

[295] In the 2.95 AVS plugin the header sits directly before its first module in static data and its description and get_module members are zero in the static image (filled at run time).

### 5.2 Module layout (2360 bytes = 0x938)

[295] Verified in the 2.95 AVS plugin's static data: description pointer at 0, host-filled members zero, latency 14, delay 14, 2 spectrum rows, 2 waveform rows, then four code pointers at the offsets below and a zero user pointer.

| # | Offset | Neutral name | Size | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|---|
| 0 | 0x000 | description | 4 | pointer to ANSI string | plugin | Module name for the Vis list. |
| 1 | 0x004 | parent_window | 4 | window handle | host | Winamp main window; set before `init` and `config`. |
| 2 | 0x008 | dll_instance | 4 | module handle | host | |
| 3 | 0x00C | sample_rate | 4 | int | host | Stream sample rate in Hz; 44100 before any `vis_set_format`; updated live afterwards. |
| 4 | 0x010 | channels | 4 | int | host | Stream channel count; default 2. |
| 5 | 0x014 | latency_ms | 4 | int | plugin | How far ahead of "now" the plugin wants its data: the host looks for the frame at output position + this value. Seen: 14 (AVS), 0 (MilkDrop). |
| 6 | 0x018 | delay_ms | 4 | int | plugin | Pause in ms the host inserts after each `render`. Seen: 14 (AVS), 10 (MilkDrop). |
| 7 | 0x01C | spectrum_rows | 4 | int | plugin | Spectrum rows wanted: 0, 1 or 2. |
| 8 | 0x020 | waveform_rows | 4 | int | plugin | Waveform rows wanted: 0, 1 or 2. |
| 9 | 0x024 | spectrum_data | 1152 | 2 x 576 unsigned bytes | host | Valid for the first `spectrum_rows` rows. |
| 10 | 0x4A4 | waveform_data | 1152 | 2 x 576 unsigned bytes | host | Valid for the first `waveform_rows` rows. |
| 11 | 0x924 | config | 4 | function (module pointer) | plugin | Configuration dialog. |
| 12 | 0x928 | init | 4 | function (module pointer) returning int | plugin | Create windows and resources. 0 = success; nonzero aborts the start and the DLL is released. |
| 13 | 0x92C | render | 4 | function (module pointer) returning int | plugin | Draw one frame from the data arrays. 0 = continue; nonzero ends the visualization (the host then calls `quit`). |
| 14 | 0x930 | quit | 4 | function (module pointer) | plugin | Destroy windows and resources. |
| 15 | 0x934 | user_data | 4 | pointer | plugin | Free for the plugin's use. |

Data notes (S08 has full detail):

- Each row has exactly 576 entries.
- Waveform rows hold signed 8-bit amplitudes (two's complement, 0 = silence) in the 5.x host for 16-bit input [5X]; the MilkDrop consumer decodes them as signed. The 2.95 convention is [UNV].
- Spectrum rows hold magnitude bytes 0..255 in 5.x; only the first 512 entries carry FFT-derived data and the remaining entries are filled with a decaying continuation [5X].
- For a mono source and a plugin asking for 2 rows, the host duplicates the row into the second. If the source has more channels than requested rows, extra channels are dropped; a mono request averages all channels [5X].

### 5.3 Lifecycle and cadence

1. The user picks a module in Preferences, from the main window's vis button, or via the IPC "execute plugin" message. Winamp stops any running visualization first. [SDK] [5X]
2. Winamp creates a dedicated **vis thread**. On it the host loads the DLL, resolves `winampVisGetHeader`, calls it, calls `get_module(index)`, writes sample rate, channels, parent window and instance handle into the module, then calls `init(module)`. A null header, a null module or a nonzero `init` result unloads the DLL and stops. [5X]
3. After a successful `init` the host sizes its frame ring from the plugin's row counts (each frame is (spectrum_rows + waveform_rows) x 576 bytes plus a 4-byte timestamp) and, if the user chose to disable the built-in visualization while a plugin runs, turns off the classic analyzer. [5X]
4. Render loop on the vis thread, until stopped:
   a. If a window message is pending, dispatch it normally. The window the plugin registered for embedding receives dialog-style handling; key messages for that window are not pre-processed; mouse-wheel events over windows of other threads are forwarded to the main window. A quit message ends the loop. [5X]
   b. Otherwise, if not paused: when a track is playing, fetch the frame nearest to (input plugin position + `latency_ms`) and copy `spectrum_rows` rows then `waveform_rows` rows into the module arrays; if no frame is available, or nothing is playing, zero both arrays completely. Call `render` while a track is playing, and once more right after playback stops (a last frame with zeroed data), then not again until playback restarts. A nonzero return ends the loop. Finally sleep `delay_ms` ms; render time is not subtracted, so the real interval is render time plus `delay_ms`.
   c. If paused: sleep the smaller of 1 ms and `delay_ms`, keep dispatching messages, make no `render` calls and no data refresh.
5. On leaving the loop (nonzero `render`, quit message or user stop): free the frame ring, re-enable the classic analyzer per the user setting, call `quit(module)` on the vis thread, then free the DLL. A stop requested from another thread sets a kill flag and waits, pumping UI messages, in 10 ms steps for up to about 2 seconds, then waits indefinitely. [5X] The 2.95 host imports a thread-termination API, so it may kill hung vis threads [295 import, behavior UNV].
6. The vis thread priority comes from a five-level user setting: lowest, below normal, normal, above normal, highest (`config_visplugin_priority`). [5X] [295: setting name present]
7. `config(module)` is called from the UI thread when the user presses Configure. The module's parent-window member is already valid then. [UNV for 2.95; some plugins read it inside `config`]

Visualization window conventions [SDK]:

- A visualization window with focus should pass keystrokes through to Winamp's main window so playback stays controllable, except keys the plugin uses itself.
- Plugins should store settings under the INI directory obtained by IPC (S10), in their own file in the Plugins folder.
- The plugin owns its window (or embeds one via the embed API, S11). The host never creates a window for it.
- One visualization is active at a time. [SDK] [5X]
- [5X-ONLY] IPC 611 registers a "vis command window" that receives next/previous/random/fullscreen/config/menu commands (command ids 40382..40391); MilkDrop 2.x uses it.

`config_visplugin_autoexec` [295 string present] makes the chosen visualization start when Winamp starts or playback begins.

## 6. DSP plugin (`winampDSPHeader` / `winampDSPModule`)

2.95 ships no DSP plugin in the stock or full installers [295], although its host holds the `DSP_*.DLL` pattern and the settings `config_dspplugin_name` / `config_dspplugin_num` [295]. The layouts below come from the public headers and the 5.x host and have not been checked against a 2.x-era DSP DLL [SDK].

### 6.1 Header (16 bytes in 5.x; 12 bytes in 2.x)

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | version | int | plugin | 0x20 for 2.x. 5.x headers use 0x21/0x22 with the main-window-taking entry point [5X-ONLY]. The 5.x host's acceptance test is lenient: it accepts versions in a range above the compiled-in value [5X]. |
| 1 | 0x04 | description | pointer to ANSI string | plugin | |
| 2 | 0x08 | get_module | function (int index) returning module pointer | plugin | Null if out of range. |
| 3 | 0x0C | key_check | function (int key) returning int | plugin | [5X-ONLY] absent from 2.x headers. |

### 6.2 Module (32 bytes)

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | description | pointer to ANSI string | plugin | |
| 1 | 0x04 | parent_window | window handle | host | Set before `init`. |
| 2 | 0x08 | dll_instance | module handle | host | |
| 3 | 0x0C | config | function (module pointer) | plugin | |
| 4 | 0x10 | init | function (module pointer) returning int | plugin | 0 on success. |
| 5 | 0x14 | process | function (module pointer, pointer to 16-bit samples, int frames, int bits_per_sample, int channels, int sample_rate) returning int | plugin | Process in place; returns the frame count to output (between half and double the input). Block sizes of at least 128 frames are expected, but not guaranteed. |
| 6 | 0x18 | quit | function (module pointer) | plugin | |
| 7 | 0x1C | user_data | pointer | plugin | |

### 6.3 Lifecycle

1. At startup (and when the selection changes) the host loads the DLL named in `config_dspplugin_name`, resolves `winampDSPGetHeader2`, calls it, calls `get_module(config_dspplugin_num)`, sets the parent-window and instance members, and calls `init`. A null header or module unloads the DLL. [5X]
2. Only one DSP module is active at a time (one stored file name and index). [5X] [295: single setting pair]
3. Input plugins reach the DSP through `dsp_is_active` and `dsp_process`. Processing is synchronous on the input plugin's decode thread. [SDK]
4. `quit` is called when the DSP is deselected and at exit; the DLL is freed right after. [5X]
5. Because the buffer is processed in place, the input plugin must provide room for up to twice the frames it passes when `dsp_is_active` is nonzero. [SDK]

## 7. General purpose plugin (`winampGeneralPurposePlugin`)

### 7.1 Layout (28 bytes) [295: checked on the media-library plugin]

| # | Offset | Neutral name | Neutral type | Filled by | Meaning |
|---|---|---|---|---|---|
| 0 | 0x00 | version | int | plugin | 0x10 in 2.95 [295]; 0x11 allowed in 5.64+ (wide description) [5X-ONLY]. |
| 1 | 0x04 | description | pointer to ANSI string | plugin | |
| 2 | 0x08 | init | function () returning int | plugin | Called after the host fills parent window and instance handle. 0 = success. In 5.66 a nonzero return unloads the plugin and marks it NOT LOADED [5X-ONLY]; the role of the return value in 2.95 is [UNV]. |
| 3 | 0x0C | config | function () | plugin | Open configuration (no arguments). |
| 4 | 0x10 | quit | function () | plugin | Called at exit. |
| 5 | 0x14 | parent_window | window handle | host | Winamp main window. |
| 6 | 0x18 | dll_instance | module handle | host | |

### 7.2 Lifecycle

1. The host scans `GEN_*.DLL`, loads each, resolves `winampGetGeneralPurposePlugin`, calls it, checks the version, sets parent window and instance handle, then calls `init`. [5X, pattern 295]
2. 5.x preloads a fixed list of its own general purpose plugins first (including the media library); 2.95's full installer also ships the media library plugin [295]. 2.95 order is [UNV].
3. A general purpose plugin typically subclasses the main window procedure in `init`, installs timers, adds menu items via IPC (S10) or creates windows (embedded via S11).
4. At exit the host calls `quit` in reverse load order and does not free the DLLs [5X].
5. `config` is invoked on the UI thread when the user presses Configure.

All calls to general purpose plugins happen on the UI thread; the plugin may create its own threads [5X].

## 8. Cross-cutting behaviors that 2.95-era plugins rely on

1. **Window classes.** Main window `Winamp v1.x`. Other class names seen in the 2.95 host: `Winamp EQ`, `Winamp PE` (playlist editor), `Winamp MB` (minibrowser), `Winamp Video`, `Winamp Gen` (frames for embedded windows) [295].
2. **IPC channel.** The user message `WM_USER` (public name `WM_WA_IPC`) sent to the main window with the IPC number in the second parameter and the argument in the first. The mechanism, `WM_COPYDATA` for file enqueue and `WM_COMMAND` ids are cataloged in S10. IPC sent from another thread is marshalled by the OS (the sender blocks).
3. **Version query.** IPC number 0; 2.95 returns 0x2905 (10501); 5.x returns 0x50yy. Plugins compare against values like 0x2900 to decide whether 2.9+ IPCs exist, and 0x2905 passes that threshold. The earlier "2.95 returns 0x2095" (8341) [SDK] reading is superseded: three independent sandbox runs of job `jobs/s10-ipc.json` sent WM_USER with lParam 0 (IPC number) and wParam 0 to the main window and each recorded the unsigned 32-bit reply 10501 = 0x2905 — run 20261016-022004 (which expected 8341 and measured 10501), run 20261016-023349, and run 20261016-060342 (step `s10_000_getversion` in each; see `audits/S10-ipc-replies.md` section 3). RFC 0009 (`docs/rfc/0009-ipc-version-constant.md`) corrects the crate constant `WINAMP_VERSION_295` to 0x2905 on the same evidence. The reference fact still rests on these three external measurements: the decisive cross-check is an in-process (plugin-DLL) caller, which the plugin host's answering surface (`crates/rustamp-plughost/src/ipc.rs`, merged) now makes conceivable but which has not yet been performed.
4. **Unsupported IPC numbers.** 5.x returns 1 and plugins test for it. 2.95 behavior is [UNV] (probably 0 from default window-procedure handling).
5. **INI location.** 2.9+ provides IPC calls returning the INI file path, INI directory and plugin directory; earlier plugins derive paths from the host executable path. The host's own settings file is `winamp.ini` [295 string]. See S12.
6. **Subclassing.** Plugins may subclass Winamp's windows by replacing the window procedure, so full compatibility requires real Win32 windows with replaceable procedures.
7. **Dialog parent.** Config and About windows are parented to the handle passed in (or the main window).
8. **UI work** belongs on the UI thread; decode and vis threads are the plugin's or the vis thread of 5.3.
9. **Playback state query.** IPC "is playing" returns 1 = playing, 3 = paused, 0 = stopped; some older-host code paths in visualization plugins refuse to start unless it is 1. [SDK] [free source]
10. **Time queries.** IPC "get output time": mode 0 = position ms (-1 when not playing), mode 1 = length in seconds, mode 2 = length in ms (2.9+). [SDK]
11. Classic skins show the built-in analyzer inside the main window; plugin visualizations run in separate windows.

## 9. Sample data at the plugin boundary

| Item | Fact | Tag |
|---|---|---|
| PCM passed to analyzer/vis feed calls | Interleaved, little-endian signed integers for 16-bit and wider; unsigned bytes for 8-bit | [5X] [SDK] |
| Row length | 576 bytes per spectrum row and per waveform row | [295 array sizes] |
| Rows | At most 2 per kind; row 0 = left, row 1 = right | [295 array sizes; channel order 5X] |
| Classic analyzer row | 75 bytes (spectrum), 75 bytes (oscilloscope) | [5X] |
| Output `write` chunk | At most 8192 bytes | [SDK] |
| `dsp_process` input | At least 128 frames expected | [SDK] |

## 10. 5.x-only items (not required of a 2.95-level host; 5.x-era plugins may call or export them)

| Item | Applies to |
|---|---|
| Wide-character exports: `winampGetExtendedFileInfoW`, `winampSetExtendedFileInfoW`, `winampGetExtendedRead_openW` and relatives; `winampUseUnifiedFileInfoDlg`; `winampAddUnifiedFileInfoPane` | input |
| `winampUninstallPlugin` | all kinds |
| `winampGetOutModeChange`; output version 0x11 | output |
| Input version flags 0x0F000000 and 0xF0000000 and the 39th member | input |
| Visualization header 0x102 with main-window-taking entry point; vis command window (IPC 611) and command ids 40382..40391 | visualization |
| DSP header 0x21/0x22 with main-window-taking entry point and `key_check` | DSP |
| General purpose version 0x11; init status semantics | general purpose |
| Media-library plugin entry points, modern-skin and `ml_*` plugins | media library |
| Plugin safe mode and its description naming convention | all |
| Service manager (IPC for API service, `.w5s` components) | all |

## 11. 2.95-bundled plugins (cross-check data from the static inventory) [295]

| Plugin | Entry point | Struct version | Notes |
|---|---|---|---|
| in_mp3 | winampGetInModule2 | 0x100 | also exports the extended-info functions and a decoder factory; seekable member is 0 in static data, flag bit 0 set |
| in_wave | winampGetInModule2 | 0x100 | seekable 1, flag bit 0 set |
| in_midi, in_mod, in_wm, in_nsv, in_cdda, in_dshow, in_vorbis | winampGetInModule2 | 0x100 | in_wm and in_dshow are optional components |
| out_wave, out_ds, out_disk, out_wm | winampGetOutModule | 0x10 | large constant ids; out_disk reports latency 0 |
| vis_nsfs | winampVisGetHeader | 0x101 | header filled at run time |
| vis_avs | winampVisGetHeader | 0x101 | 12-byte header followed by one module; latency 14, delay 14, 2+2 rows; also loads its own effect add-ins (a separate, private extension format outside this ABI) |
| gen_ml | winampGetGeneralPurposePlugin | 0x10 | media library host plugin; no other exports in the 2.95 build |
| Geiss 4.29, Geiss 2 1.0 | winampVisGetHeader | 0x101 | third-party; static header, module via `get_module` |
| in_openmpt 0.8.9 (two builds) | winampGetInModule2 | 0x100 (build for old hosts), 0x0F000100 (5.x build) | |
| MilkDrop 2 (from Winamp 5.66) | winampVisGetHeader | uses the main-window-taking 5.x entry | needs several 5.x IPC calls; will not run on a 2.95-level host without extensions |

## 12. Open / unverified items (for runtime verification against a running 2.95)

| ID | Question |
|---|---|
| U-1 | Confirm that posting message 0x0402 ends the track and advances the playlist in 2.95, and list other private WM_USER+n messages the host accepts from plugins. |
| U-2 | Does 2.95 run visualizations on a separate thread as 5.x does, and which thread calls `config`, `init` and `quit`? (The host imports thread creation, priority and termination APIs; ownership of each call is unconfirmed.) |
| U-3 | Sign convention and offset of waveform bytes and scaling of the 576-entry spectrum in 2.95 (capture with a test visualization and a known tone). |
| U-4 | Does 2.95 overwrite the output module `id` as 5.x does, or compare ids directly; how the selection is stored (`config_outname` appears to hold a file name). |
| U-5 | Visualization library discovery pattern in 2.95 (prefix filter or any DLL exporting the entry point). |
| U-6 | Return value for unknown IPC numbers in 2.95. |
| U-7 | Acceptance range of version fields per kind in 2.95 (strict equality or range). |
| U-8 | Role of the general purpose `init` return value in 2.95. |
| U-9 | Whether 2.95 passes any argument to `winampDSPGetHeader2` and `winampVisGetHeader`. |
| U-10 | Bit depths accepted by the two PCM feed calls in 2.95 (8 and 16 expected). |
| U-11 | Ring sizing and time-matching tolerances in 2.95 (5.x numbers are in 3.8). |
| U-12 | Order in which `dsp_process` applies the DSP plugin and the equalizer. |
| U-13 | Exact signatures of the optional extended-info exports in 2.95. |
| U-14 | Whether the 2.95 host filters `file_extensions` list elements by length and how it treats empty pairs. |
| U-15 | Calling context of the general purpose `config` in 2.95 (modal? parent?). |
| U-16 | Which input-module callbacks the 2.95 host tolerates as null (for example `is_our_file`, `eq_set`, `set_info` consumers); plugins observed in 2.95 fill every callback slot. |
| U-17 | Encoding and value range of the 75-byte analyzer rows used by the advanced analyzer path (belongs with S08). |
| U-18 | How the 5.x host slices PCM into spectrum windows (window size and step) and whether 2.95 does the same; delegated to S08. |
| U-19 | Whether the file-extensions pointer is allowed to be null in static data (the two plugins examined hold a non-null pointer). |

## 13. Coverage checklist

| Area | Status |
|---|---|
| Entry-point names and signatures | Done; names [295]; main-window variants are [5X-ONLY]. |
| Input module layout | Done; [295] checked on two 2.95 plugins. |
| Output module layout | Done; [295] checked on two 2.95 plugins. |
| Visualization header/module layout | Done; [295] checked on the AVS plugin and a second library header. |
| DSP header/module layout | Done from the public headers and 5.x; no 2.95 DSP DLL to check [SDK]. |
| General purpose layout | Done; [295] checked on the media-library plugin. |
| Version constants | Done; acceptance rules partly [UNV]. |
| Calling convention | Done. |
| Lifecycle and call order | Done for 5.x; 2.95 inferred. |
| Threading | Done for 5.x; 2.95 partly [UNV]. |
| Analyzer/vis feed callbacks | Done at contract level; numerical processing in S08. |
| DSP hooks | Done at contract level. |
| Vis cadence, latency and delay | Done for 5.x; field semantics confirmed by real values; 2.95 timing [UNV]. |
| Config/About and parent handle | Done. |
| Host behaviors plugins rely on | Done (section 8) with pointers to S10. |
| 5.x-only list | Done (section 10). |
