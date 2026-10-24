# BETA — First-user readiness review

**Date:** 2026-10-17. **Reviewed revision:** `f9012dc63b35b92edb83da5f19609002e41a94f8`.
**Scope:** static, clean-room review; findings only, no implementation changes.
**Working directory / write boundary:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave46-beta-readiness` only.

## Decision

**Not ready for an ordinary first-user beta.** The default skin is solid-magenta placeholder artwork, the visible file-opening paths cannot choose an ordinary media file, and failed media opens can leave the window apparently playing silence without an error message. Packaging also omits an explicit CRT deployment policy. These are product-path failures, not compilation failures (BETA-01 through BETA-05 below).

Severity: **P1** blocks basic first use or loses meaningful failure feedback; **P2** is a misleading/broken exposed feature. This is a source-level assessment, not a new VM acceptance result.

## 1. Journey — one line per step

| Step | What the first user reaches | Evidence |
|---|---|---|
| Unpack | A release player folder and a separate host folder, not an installer; no first-run guide or external default skin is copied. | `xtask/src/package.rs:128-197`, `xtask/src/package.rs:219-225` |
| Launch without arguments | On a working device, construct an empty stopped engine, then open the main window **and an unconditional playlist window**; output opening happens before either window. | `crates/rustamp-app/src/app.rs:160-182`, `crates/rustamp-audio/src/engine.rs:223`, `crates/rustamp-app/src/lib.rs:110-145`, `crates/rustamp-app/src/window.rs:1149-1198` |
| Launch with a file | Queue one path and submit Play before opening the window; decoder work then happens on a worker thread. | `crates/rustamp-app/src/app.rs:180-182`, `crates/rustamp-app/src/lib.rs:141-145`, `crates/rustamp-audio/src/engine.rs:1031-1075` |
| Launch with a bad media path | The window can still open: sniff/open produces an error, but the worker exits without EndOfStream and the engine remains Playing; neither app nor UI displays the error (BETA-04). | `crates/rustamp-core/src/audio.rs:122`, `crates/rustamp-audio/src/decode.rs:420-425`, `crates/rustamp-audio/src/engine.rs:923-934`, `crates/rustamp-app/src/app.rs:496-534` |
| Launch without configuration | Read `%RUSTAMP_CONFIG%`, otherwise `%APPDATA%\Rustamp\rustamp.toml`; missing path/file means defaults, no warning, no first-run config creation. | `crates/rustamp-app/src/config.rs:43-98` |
| Launch with malformed/unreadable configuration | Use defaults and print a useful reason to stderr, rather than abort. | `crates/rustamp-app/src/config.rs:75-95`, `crates/rustamp-app/src/lib.rs:83-87` |
| Launch with a missing configured skin | CLI skin wins over config; a load error falls back to the built-in placeholder and prints the failed path/reason to stderr, without aborting. | `crates/rustamp-app/src/skin.rs:29-34`, `crates/rustamp-app/src/skin.rs:92-113`, `crates/rustamp-app/src/lib.rs:94-99` |
| Launch without an audio device | Default launch fails **before a window**, with an output error and failure exit; there is no automatic silent fallback. `--no-audio` is an explicit windowed workaround and `--null` is headless. | `crates/rustamp-output/src/lib.rs:57-65`, `crates/rustamp-output/src/cpal_out.rs:145-153`, `crates/rustamp-output/src/cpal_out.rs:274-279`, `crates/rustamp-app/src/app.rs:160-173`, `crates/rustamp-app/src/main.rs:37-42`, `crates/rustamp-app/src/cli.rs:55-59` |
| See the first window | The model correctly selects the non-scrolling program name `RUSTAMP`, blank stopped time/kbps/kHz and unknown channels; the default artwork/font is not usable art (BETA-02). | `specs/S02-main-window-behaviour.md:259`, `specs/S02-main-window-behaviour.md:285`, `crates/rustamp-ui/src/behaviour.rs:26-31`, `crates/rustamp-ui/src/behaviour.rs:173-180`, `crates/rustamp-ui/src/behaviour.rs:510-539`, `crates/rustamp-skin/src/skin.rs:76-104` |
| Click to open media | Eject emits no command; playlist ADD opens a keyboard-only stand-in with no picker and no punctuation input, so normal `C:\Music\song.mp3` input cannot be completed (BETA-03/BETA-06). | `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-app/src/playlist_window.rs:214-219`, `crates/rustamp-ui/src/dialogs.rs:34-52`, `crates/rustamp-app/src/window.rs:776-805` |
| Hear a file | The practical shipped route is `rustamp <file>`; once a queue entry exists, playlist double-click or Play reaches the engine, decoder, ring and output, subject to the silent failure paths below. | `crates/rustamp-app/src/lib.rs:141-145`, `crates/rustamp-app/src/playlist_window.rs:129-143`, `crates/rustamp-app/src/app.rs:1125-1129`, `crates/rustamp-audio/src/engine.rs:1031-1075` |
| Lose the device | Backend failure latches are not exposed by the output trait or polled by the engine, so playback can remain Playing without audio (H4; BETA-05). | `crates/rustamp-output/src/wasapi_out.rs:317-325`, `crates/rustamp-output/src/cpal_out.rs:99-104`, `crates/rustamp-core/src/audio.rs:320-353`, `crates/rustamp-audio/src/engine.rs:825-856` |
| Close and return | Native main-window close quits, but skinned Close and menu Exit do not; config loading has no production save path, so UI changes are not written back. | `crates/rustamp-app/src/window.rs:1225-1229`, `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-ui/src/menu.rs:424-430`, `crates/rustamp-app/src/menu.rs:119-121`, `crates/rustamp-app/src/config.rs:43-98`, `crates/rustamp-app/src/app.rs:960-1069` |

### Launch diagnostics and panic boundary

- Argument parsing prints the rejected option/value plus usage and exits 2; ordinary startup errors print `rustamp: {error}` and exit failure. Neither is an in-window error presentation (`crates/rustamp-app/src/main.rs:24-42`, `crates/rustamp-app/src/cli.rs:83-94`).
- Headless `--null` is different from the GUI error path: it prints “no input file” and returns when empty, and returns the stored decoder error at the end of a failed run (`crates/rustamp-app/src/app.rs:813-817`, `crates/rustamp-app/src/app.rs:863-865`, `crates/rustamp-app/src/app.rs:879-881`).
- Release GUI launch is not blocked by the debug-only VM opt-in; `RUSTAMP_NO_GUI` still rejects it. A mistakenly packaged debug build normally refuses on a user's machine (`crates/rustamp-app/src/window.rs:162-181`, `xtask/src/package.rs:233-235`).
- Missing config and missing skin use explicit fallback branches, not panic branches. There are still `expect` panic sites for an invariant failure loading the built-in skin and inability to spawn the decoder thread; **panic-free startup is not proven** (`crates/rustamp-app/src/config.rs:84-95`, `crates/rustamp-app/src/skin.rs:78-81`, `crates/rustamp-app/src/skin.rs:103-110`, `crates/rustamp-audio/src/decode.rs:323-335`).

## 2. Findings, in severity order

### P1 — BETA-01: The package has no self-contained runtime policy (X1 / runtime packaging finding)

**User impact:** on a fresh Windows installation lacking the required VC runtime, an EXE can fail at the Windows loader before Rustamp's diagnostics run. The packager only copies EXEs and selected license/manifest files; its build command neither adds `+crt-static` nor stages CRT DLLs/installers (`xtask/src/package.rs:128-197`, `xtask/src/package.rs:289-313`). The VM runner explicitly documents the observed missing-runtime failure `0xC0000135`, stages x64 CRT DLLs, and **separately statically links the i686 host** because x86/x64 CRT filenames collide beside the player (`docs/notes/vm-run-tests.ps1:82-108`). That protection is absent from `package`.

**Limit:** these exact release binaries were not built or inspected; caller-provided compiler flags or a preinstalled system runtime can change the outcome. This is a concrete deployment gap/conditional startup blocker, not a claim that every clean Windows machine necessarily lacks those DLLs. Card: `docs/tasks/X1-xtask-package.md:18-30`; historical runtime finding: `docs/HANDOFF.md:47`.

### P1 — BETA-02: The first-run/default/fallback skin is flat magenta, including the font (art lane J / C1)

**User impact:** no configured skin does not yield a recognizable classic player with legible labels. The app loads an empty memory source using `BuiltinDefaultSkin`; it generates every sheet as a solid `[255,0,255,255]` rectangle (`crates/rustamp-app/src/skin.rs:78-81`, `crates/rustamp-skin/src/skin.rs:76-104`). The renderer obtains its bitmap font from that same Text sheet and copies glyphs opaquely, so setting model text to `RUSTAMP` does **not** make it readable with these assets (`crates/rustamp-ui/src/render.rs:631-646`, `crates/rustamp-ui/src/render.rs:663-686`). No actual default-skin assets are staged (`xtask/src/package.rs:128-153`). Lane J is the planned shipped-art responsibility (`docs/PLAN.md:149-151`); the app explicitly admits the placeholder ships today (`crates/rustamp-app/src/skin.rs:4-11`).

### P1 — BETA-03: There is no ordinary GUI route from an empty queue to choosing a media file (N1/N2/N3)

**User impact:** clicking Eject does nothing; ADD opens a stand-in rather than a file browser. The open-media field accepts letters/digits/spaces only, no path separators, drive colon or extension dot; modified keys are sent to global shortcuts, so there is no paste path here either (`crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-app/src/playlist_window.rs:214-219`, `crates/rustamp-ui/src/dialogs.rs:425-478`, `crates/rustamp-app/src/window.rs:756-773`, `crates/rustamp-app/src/window.rs:1508-1521`). Pointer input in dialogs is ignored, and main/playlist event handlers do not implement dropped-file loading (`crates/rustamp-app/src/window.rs:776-805`, `crates/rustamp-app/src/window.rs:1071-1144`, `crates/rustamp-app/src/window.rs:1225-1317`).

**Shortest intended chain, not a completed human flow:** ADD -> OpenMedia -> staged/typed path -> Enter -> AddItems (`play:false`) -> playlist double-click -> PlayEntry -> `Engine::play_index` -> decoder worker -> output callback (`crates/rustamp-app/src/playlist_window.rs:214-219`, `crates/rustamp-ui/src/dialogs.rs:528-584`, `crates/rustamp-app/src/dialogs.rs:140-146`, `crates/rustamp-app/src/app.rs:1101-1129`, `crates/rustamp-audio/src/engine.rs:1031-1075`). Programmatic staging is not a native picker: the shipped input handlers never invoke it (`crates/rustamp-app/src/dialogs.rs:166-169`, `crates/rustamp-app/src/window.rs:776-805`). Directory confirmation adds the directory as one file, without enumeration; URL opening is explicitly unsupported by the decoder (`crates/rustamp-ui/src/dialogs.rs:566-576`, `crates/rustamp-decoders/src/symphonia_dec.rs:275-279`).

### P1 — BETA-04: Missing/unsupported media can leave phantom playback; errors disappear at the GUI boundary (N3 / engine lifecycle)

**User impact:** launch with a missing path or unsupported codec can produce a window that claims to play silence indefinitely. `start_run` sets Playing, but an opening failure reports an error and breaks without sending EndOfStream; the worker wrapper only sends Stopped/Returned (`crates/rustamp-audio/src/engine.rs:1073-1075`, `crates/rustamp-audio/src/decode.rs:420-425`, `crates/rustamp-audio/src/decode.rs:325-333`). `on_decode_out(Error)` only records/publishes the error and `Stopped` only clears the sender; transport completion requires EndOfStream, so neither stops this failed run (`crates/rustamp-audio/src/engine.rs:843-847`, `crates/rustamp-audio/src/engine.rs:923-934`). Play then returns success without restarting because the engine still considers itself Playing (`crates/rustamp-audio/src/engine.rs:621-623`).

**Silence boundary:** the app drains events, but its handler and the UI handler both ignore `Event::Error`; it is neither rendered nor turned into a note for stderr (`crates/rustamp-app/src/app.rs:444-447`, `crates/rustamp-app/src/app.rs:496-534`, `crates/rustamp-ui/src/behaviour.rs:231-275`). Synchronous engine command failures also become only Error events (`crates/rustamp-app/src/app.rs:473-479`, `crates/rustamp-app/src/app.rs:944-946`). Thus even pressing Play on an empty queue can optimistically light Playing while the real engine rejected index 0 (`crates/rustamp-ui/src/behaviour.rs:60-62`, `crates/rustamp-ui/src/behaviour.rs:386-389`, `crates/rustamp-audio/src/engine.rs:985-988`).

**Codec scope:** the registry contains only Symphonia, with the enabled MP1/2/3, FLAC, Vorbis, WAV/PCM/ADPCM, AIFF, CAF, AAC/ALAC/MP4 and Matroska support; no Opus or module-decoder fallback is registered. Unsupported format/codec creation has explicit errors, but they reach the black hole above (`crates/rustamp-decoders/src/lib.rs:7-10`, `crates/rustamp-decoders/src/lib.rs:26-29`, `crates/rustamp-decoders/Cargo.toml:13`, `crates/rustamp-core/src/audio.rs:278-290`, `crates/rustamp-decoders/src/symphonia_dec.rs:313-327`). This does not certify that every file in an enabled container is decodable.

### P1 — BETA-05: Device failure is not recoverable/reportable end to end (H3/H4, RFC 0012)

**User impact:** unplugging/changing an endpoint or a failed WASAPI write can stop sound while the engine remains Playing and decode waits on a full ring. WASAPI and cpal have local latches, but `OutputDevice` has no health accessor and `Engine::poll` reads no output-health status (`crates/rustamp-output/src/wasapi_out.rs:317-325`, `crates/rustamp-output/src/wasapi_out.rs:720-725`, `crates/rustamp-output/src/cpal_out.rs:318-320`, `crates/rustamp-core/src/audio.rs:320-353`, `crates/rustamp-audio/src/engine.rs:825-856`). H4 is **design only**, not an implemented fix (`docs/tasks/H4-output-error-rfc.md:3-4`, `docs/tasks/H4-output-error-rfc.md:31-35`).

**Initial refusal/retry:** opening an unavailable device aborts startup with stderr, not a panic or in-window remedy (journey above). cpal stream construction/start can also fail on the first Play; the engine has already taken ownership of its sole render callback before `output.start` returns an error, so a later first-start retry finds `render callback missing` rather than rebuilding it (`crates/rustamp-output/src/cpal_out.rs:310-341`, `crates/rustamp-audio/src/engine.rs:1063-1069`). That error is swallowed by the same GUI Error-event gap as BETA-04.

### P1 — BETA-06: Familiar primary controls are hit-testable but dead (D2/N4/S04)

**User impact:** skinned Minimize, Shade, Close, Eject, clutter O and clutter V can show pressed/hint state but emit nothing: none has a release arm, so all fall through to `None` (`crates/rustamp-ui/src/widget.rs:155-173`, `crates/rustamp-ui/src/behaviour.rs:188-192`, `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-ui/src/behaviour.rs:558-568`). The main-menu Play/Options/Playback/Visualization/Skins parents do not open submenus; Exit closes **only the popup**, not Rustamp (`crates/rustamp-app/src/menu.rs:119-129`, `crates/rustamp-ui/src/menu.rs:308-314`, `crates/rustamp-ui/src/menu.rs:390-430`). Notes for deferred operations go to stderr, not a visible explanation (`crates/rustamp-app/src/window.rs:1329-1331`). Native main Close and command-driven minimize/shade remain different, working paths (`crates/rustamp-app/src/window.rs:1225-1229`, `crates/rustamp-app/src/app.rs:984-999`, `crates/rustamp-app/src/window.rs:339-366`).

### P2 — BETA-07: Playlist visibility/check marks do not control its OS window (S05/S06/N4)

**User impact:** the playlist opens even though the initial model says closed; its toggle changes a flag/check but cannot hide it, and its native close explicitly keeps it open (`crates/rustamp-ui/src/state.rs:145-146`, `crates/rustamp-ui/src/behaviour.rs:410-413`, `crates/rustamp-app/src/window.rs:1198`, `crates/rustamp-app/src/window.rs:1071-1075`). There is no playlist counterpart to EQ's visibility-controlled creation/destruction (`crates/rustamp-app/src/window.rs:376-385`, `crates/rustamp-app/src/window.rs:1001-1037`, `crates/rustamp-app/src/window.rs:1336-1354`).

### P2 — BETA-08: Shuffle/Repeat lights advertise playback behavior that is not applied (S15 / N3)

**User impact:** toggles update app flags and publish their lights, but the engine ignores those commands and the worker simply increments/decrements a queue index; end-of-queue produces EndOfStream, not repeat (`crates/rustamp-app/src/app.rs:968-982`, `crates/rustamp-audio/src/engine.rs:554-605`, `crates/rustamp-audio/src/decode.rs:413-418`, `crates/rustamp-audio/src/decode.rs:451-458`). The app itself calls these app-owned snapshot flags rather than engine playback modes (`crates/rustamp-app/src/app.rs:586-595`). Leave these unavailable/labeled until playback policy exists; do not ship active-looking placebo controls.

### P2 — BETA-09: Playlist-file loading is a dead end; skip diagnostics exist only at the library API (E2/E3 / RFC 0010)

**User impact today:** LIST OPTS opens `Load Playlist`, but the dialog shows an empty field and Enter has no load action. Passing a playlist as the CLI file also queues it as media, not as a list (`crates/rustamp-app/src/playlist_window.rs:221-229`, `crates/rustamp-app/src/dialogs.rs:202-205`, `crates/rustamp-app/src/dialogs.rs:124-153`, `crates/rustamp-app/src/app.rs:180-182`). No app playlist reader call is wired; this agrees with E3's caller audit (`docs/tasks/E3-playlist-skips.md:87-106`).

**Silent-drop boundary, not an invented GUI execution path:** M3U reports dangling EXTINF/unresolvable locations, PLS reports empty/orphaned/unresolvable File keys, XSPF reports missing/unresolvable locations (`crates/rustamp-playlist/src/m3u.rs:76-124`, `crates/rustamp-playlist/src/pls.rs:122-160`, `crates/rustamp-playlist/src/xspf.rs:197-214`). The new report APIs retain reasons, but legacy readers explicitly discard them through `into_entries`; an app using the legacy API would silently lose these entries (`crates/rustamp-playlist/src/io.rs:19-31`, `crates/rustamp-playlist/src/io.rs:76-84`, `crates/rustamp-playlist/src/entry.rs:113-116`). Do not describe E3 as wholly unfixed, or claim users already see a wired reader's skip warning.

### P2 — BETA-10: Exposed visualization/plugin/preferences features are skeletons, not beta-ready features (S08 / P3 / S16)

- **S08:** `rustamp-vis` is doc-only; the audio tap retains raw waveform samples but leaves spectrum zero. The main renderer draws no live visualizer (`crates/rustamp-vis/src/lib.rs:1-8`, `crates/rustamp-audio/src/vistap.rs:8-16`, `crates/rustamp-ui/src/render.rs:185-198`). The open spec card is explicit about this blocker (`docs/tasks/S08-vis.md:3-6`).
- **P3/M3:** the bundled host serves a window/control channel but loads no real DLL and ignores LoadPlugin; configuring/toggling a vis plugin only records a note (`crates/rustamp-plughost/src/main.rs:3-7`, `crates/rustamp-app/src/app.rs:1064-1065`). The acceptance card is a blocked design, not compatibility evidence (`docs/tasks/P3-dll-acceptance.md:1-5`).
- **Preferences:** only two visualization page identities exist and the dialog draws only the selected name, no settings controls (`crates/rustamp-prefs/src/lib.rs:21-22`, `crates/rustamp-prefs/src/lib.rs:44-60`, `crates/rustamp-app/src/dialogs.rs:213-216`). EQ presets likewise only record an unimplemented note (`crates/rustamp-app/src/app.rs:365-379`). These must be explicitly excluded or disabled, not sold as functioning options.

### P2 — BETA-11: Platform integration is still a stub apart from DPI (F1 / M2)

**User impact:** no implemented single-instance forwarding, global media keys/hotkeys, taskbar integration, file associations or player-side external-control window comes from this crate; it exports only DPI/chrome helpers (`crates/rustamp-platform/src/lib.rs:4-15`). The real app entrypoint directly constructs and runs a new App, not a forwarding path (`crates/rustamp-app/src/lib.rs:74-145`); therefore a second supported launch is not forwarded into the existing queue. Do not conflate the plugin host's hidden window with complete player-side platform integration (`crates/rustamp-plughost/src/main.rs:9-12`). These are planned M2 features (`docs/PLAN.md:171-177`), not shipped functionality.

### P2 — BETA-12: Config is read-only and several apparent settings are not consumed (N1 / S12/S16)

**User impact:** volume/skin/window-mode changes are not saved on exit; no first-run TOML is generated. Production config code only resolves, parses and reads, and the app changes in-memory fields/skin rather than saving (`crates/rustamp-app/src/config.rs:43-98`, `crates/rustamp-app/src/app.rs:960-1086`, `crates/rustamp-app/src/window.rs:1225-1229`). Editing `audio.output`/`device` does not select them: App chooses solely from CLI OutputChoice and constructs EngineConfig with volume/balance plus engine defaults. Saved EQ state is also not applied in construction (`crates/rustamp-core/src/config.rs:63-79`, `crates/rustamp-core/src/config.rs:147-153`, `crates/rustamp-app/src/app.rs:160-179`, `crates/rustamp-audio/src/engine.rs:251-254`). A sample config must distinguish supported keys from aspirational ones.

## 3. What is clickable, before loading anything

- **Actually routed:** transport Prev/Play/Pause/Stop/Next, volume/balance, EQ toggle, playlist **model** toggle, A/D and window-menu button; time-display click is UI-local. Seek emits nothing without known duration; I intentionally does nothing with no track (`crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-ui/src/behaviour.rs:499-506`). Working dispatch does not override the empty-queue/Error or phantom-visibility findings above.
- **Not equivalent to a working feature:** Shuffle/Repeat only change flags; Eject/O/V and skinned Minimize/Shade/Close have no release command; menu submenu arrows and Exit are not functional navigation/exit (`crates/rustamp-app/src/app.rs:968-982`, `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-app/src/menu.rs:119-129`).
- **Presenter failures can be silent too:** resizing/acquiring/presenting the main softbuffer frame can fail without a user message; initial window/context/surface failures print to stderr and exit the event loop, but are not propagated as a presenter error after `run_app` succeeds (`crates/rustamp-app/src/window.rs:579-600`, `crates/rustamp-app/src/window.rs:1158-1179`, `crates/rustamp-app/src/window.rs:236-238`). No graphical failure was injected in this review.

## 4. Packaged-layout answer

For default `cargo xtask package`, **assuming a fresh output directory**, the code stages the following under `target/package` (release is the default):

```text
manifest.json
player/
  rustamp.exe                    x86_64-pc-windows-msvc
  rustamp-plughost32.exe          i686-pc-windows-msvc
  LICENSE                        GPL
  PLUGIN-EXCEPTION.md
  manifest.json
host/
  rustamp-plughost32.exe          second copy, independently packaged
  LICENSE-MIT
  LICENSE-APACHE
  manifest.json
```

Evidence for targets/default/output path: `xtask/src/package.rs:21-27`, `xtask/src/package.rs:43-47`, `xtask/src/package.rs:219-225`; exact copies/writes: `xtask/src/package.rs:128-197`. `--player-only` omits building/bundling the host and explicitly permits detached operation; it is not the same artifact (`xtask/src/package.rs:100-122`, `xtask/src/package.rs:165-169`). Existing output directories are reused, not cleaned, so this is not a guarantee that stale extra files are removed (`xtask/src/package.rs:98`, `xtask/src/package.rs:129-132`, `xtask/src/package.rs:175-178`).

| Item a recipient might expect | Will it be there? / consequence | Evidence |
|---|---|---|
| README / quick start / beta limitations | **No**; no explanation of CLI-only practical media opening, audio-device requirements or missing features is staged. | Complete staging blocks: `xtask/src/package.rs:128-197` |
| License files | **Yes**, GPL + exception in player, MIT/Apache in host; **not** MIT/Apache alongside the bundled host inside a separately distributed player folder. Do not say the package has no license at all. | `xtask/src/package.rs:133-147`, `xtask/src/package.rs:179-180`, `xtask/src/package.rs:525` |
| Third-party notices / source bundle or source offer | No `THIRD_PARTY.md`, dependency-license bundle, corresponding-source archive or offer is generated/copied by this command; distribution compliance and any separate source-delivery mechanism need release-owner review. | `xtask/src/package.rs:128-197`, `xtask/src/package.rs:289-313`; dependency-notice obligations: `CONTRIBUTING.md:77-83` |
| A real default skin | **No external skin**; fallback is embedded placeholder generation, not finished art. A fresh install therefore needs no skin file to start, but looks broken. | `xtask/src/package.rs:128-153`, `crates/rustamp-app/src/skin.rs:78-81`, `crates/rustamp-skin/src/skin.rs:89-104` |
| Config/sample config | **No**; absence itself is safe and intentional, not a startup failure. App reads user AppData/override, not a portable TOML beside the EXE. | `xtask/src/package.rs:128-197`, `crates/rustamp-app/src/config.rs:43-98` |
| CRT DLLs / redistributable / explicit static-runtime policy | **No** in this code path; conditional cold-machine loader failure and mixed-bitness host risk, BETA-01. | `xtask/src/package.rs:128-197`, `xtask/src/package.rs:296-302`, `docs/notes/vm-run-tests.ps1:82-108` |
| Plugins / extra codecs / installer / file associations | No plugin DLLs or installer are staged; only the Symphonia decoder registry is supplied, and the platform export is DPI-only. | `xtask/src/package.rs:128-197`, `crates/rustamp-decoders/src/lib.rs:26-29`, `crates/rustamp-platform/src/lib.rs:13-15` |

The packaging license probe is also weaker than the newer repository auditor: it asks for one host's normal dependency tree without `--all-features` and rejects only strings containing GPL/GFDL. It does not establish complete distribution-license compliance (`xtask/src/package.rs:324-375`, `xtask/src/package.rs:390-393`; broader required audit: `CONTRIBUTING.md:41-50`, `CONTRIBUTING.md:77-83`). This review supplies no legal clearance.

## 5. Known-card reconciliation: do not report historical defects as current

- **A8 is already addressed in this revision, despite its card saying “Not yet fixed”.** Current `start_run` detects unread frames, requests an epoch and passes `drain_epoch` to the worker; `write_all` awaits that drain before writing (`docs/tasks/A8-ring-reuse.md:3-4`, `crates/rustamp-audio/src/engine.rs:1008-1039`, `crates/rustamp-audio/src/decode.rs:299-321`, `crates/rustamp-audio/src/decode.rs:672-677`, `crates/rustamp-audio/src/decode.rs:828-842`). A9 records the repair (`docs/tasks/A9-ring-start-and-limiter-default.md:15-26`). **Do not ship a recurrence** of stale audio after Stop/Play; VM regression execution is still required, and was not performed here.
- **E3 already added reporting APIs**, but not app playlist loading; the legacy API remains deliberately silent (`crates/rustamp-playlist/src/io.rs:19-31`, `crates/rustamp-playlist/src/io.rs:76-84`, `docs/tasks/E3-playlist-skips.md:87-106`).
- **H4 remains an RFC/design, S08 remains a skeleton, P3 remains no real DLL loading, and platform integration remains DPI-only**; these are current gaps, with source evidence in BETA-05/BETA-10/BETA-11, not just stale HANDOFF summaries.
- **N4's “Implemented” labels overstate current click-through.** Its title-bar/Eject/O/V/Exit claims conflict with the actual release dispatch and menu mapping; use current source, not that historical status table (`docs/tasks/N4-app-window-audit.md:39-53`, `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-ui/src/menu.rs:424-430`).

## 6. The five things to fix first

1. **Make the package start on a runtime-clean VM** — define x64 runtime deployment plus self-contained i686 host, ship quick-start/notices; touches `xtask/src/package.rs` (BETA-01; `xtask/src/package.rs:128-197`, `xtask/src/package.rs:289-313`).
2. **Ship legible, newly drawn default artwork** — every first launch and missing-skin recovery currently selects magenta sheets; touches `crates/rustamp-skin/src/skin.rs` and `crates/rustamp-app/src/skin.rs` (BETA-02; `crates/rustamp-skin/src/skin.rs:76-104`, `crates/rustamp-app/src/skin.rs:78-81`).
3. **Deliver one real user file-opening path** — wire Eject/ADD to a picker or full text/drop input, not test-only staged paths; touches `crates/rustamp-app/src/window.rs`, `crates/rustamp-ui/src/dialogs.rs`, `crates/rustamp-ui/src/behaviour.rs` (BETA-03; `crates/rustamp-ui/src/dialogs.rs:34-52`, `crates/rustamp-ui/src/behaviour.rs:427-459`).
4. **Stop phantom playback and surface actionable errors, including H4** — failed files/devices must stop or recover explicitly and permit retry; touches `crates/rustamp-audio/src/engine.rs`, `crates/rustamp-audio/src/decode.rs`, `crates/rustamp-app/src/app.rs`, and the RFC-governed output contract/backends (BETA-04/05; `crates/rustamp-audio/src/engine.rs:923-934`, `crates/rustamp-app/src/app.rs:496-534`, `docs/tasks/H4-output-error-rfc.md:49-58`).
5. **Make exposed close/minimize/shade/playlist/menu controls honest** — users must be able to close what they opened; wire working actions and hide/disable unsupported ones rather than drawing placebo choices; touches `crates/rustamp-ui/src/behaviour.rs`, `crates/rustamp-app/src/window.rs`, `crates/rustamp-app/src/menu.rs` (BETA-06/07/08/10; `crates/rustamp-ui/src/behaviour.rs:427-459`, `crates/rustamp-app/src/window.rs:1071-1075`, `crates/rustamp-app/src/menu.rs:119-129`).

## 7. What this review cannot determine

- Whether the final release EXEs actually import particular CRT DLLs, have the intended architecture, launch on a truly clean OS, trigger reputation/security prompts, or work under particular endpoint drivers: package/build code is not artifact or hardware acceptance (`xtask/src/package.rs:289-313`, `docs/notes/vm-run-tests.ps1:82-108`). No package or player was executed here.
- Actual pixels, native chrome, interactive input/focus behavior, sound quality, device-loss timing and A8 regression behavior in this revision: static renderer/handler/handshake inspection is not a new GUI/audio VM run (`crates/rustamp-app/src/window.rs:225-238`, `crates/rustamp-audio/src/engine.rs:1008-1075`).
- General codec/container compatibility or any real plugin DLL compatibility: enabled codec features and an IPC surface cannot prove them (`crates/rustamp-decoders/Cargo.toml:13`, `docs/tasks/P3-dll-acceptance.md:48-61`).
- Whether a later external release process supplies notices, corresponding source, signing or installation; this command does not (`xtask/src/package.rs:128-197`).

## 8. Verification and boundary receipt

- Initial `git status --short`: empty; reviewed HEAD recorded above.
- `cargo fmt --all -- --check`: passed, no output.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed (dev profile, 37.33 s), offline; `CARGO_TARGET_DIR` explicitly set to this worktree's `target`.
- No `cargo test`, test binary, player, package command, reference binary or graphical program was run. No download, commit, git write command or code change was made. Cargo's ignored check artifacts stayed under this worktree's `target`; the only source-controlled-tree deliverable is this card.
- No forbidden canonical/research/reference tree was inspected or modified; repository instructions, cards, specs and Rustamp's own source were read only from this worktree. No third-party code copied.
- Final `git status --short` receipt (inside the worktree): `?? docs/tasks/BETA-readiness.md`; no tracked source modification.
