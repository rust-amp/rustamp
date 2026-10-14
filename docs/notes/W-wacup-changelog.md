# W-wacup-changelog: research notes from the WACUP changelog

Status: inspiration only (see `CLEANROOM.md`, section 1). WACUP is closed source. Everything here is paraphrased in our own words. No code, assets or verbatim changelog text. Entries are referenced by version and date only.

Provenance and coverage:
- Source: https://getwacup.com/changelog/ (index, about 300 x86 and x64 entries from 0.9.9.1316 on 6 Nov 2016 to 1.99.55.25130 on 30 Sep 2026), plus the preview index.
- Read in full: 1.99.55.25130 (x86), 1.99.54.24960, 1.99.50.24496, 1.99.47.23960, 1.99.22.20200, 1.99.3.18106, 1.8.0.12774, 1.0.9.4994, 0.9.9.1316 (nine entries across 2016 to 2026).
- Not read: the remaining entries. The lists below are a sample, not a full audit. Treat "not mentioned" as "not in the sampled entries".
- Spec tags: S09 plugin ABI, S10 IPC/commands, S11 embed windows, S08 vis, S17 skins, S18 output.

---

## 1. Plugin compatibility quirks (26 items)

Format: area, issue, implication for Rustamp, tag.

1. **AVS (general), 1.0.9 (Jan 2020) and 1.8.0 / 1.99.54 (Aug 2026)**: AVS is named as crash-prone, bundled with several versions, and was the subject of a scaled-frame regression fix. Implication: run vis plugins in the 32-bit host process and treat their crashes as recoverable. Tag: S08, S11.
2. **MilkDrop, 1.0.9 and 1.8.0 / 1.99.54**: preset compile failures crashed the host, and preset crashes were fixed later. Text scaling under OS text scaling was also broken. Implication: a bad preset must never take the host down, and vis windows must follow OS scaling. Tag: S08.
3. **Vis data and multi-module setups, 1.99.54 (Aug 2026)**: a data regression hit visualisation modules, and multi-module vis DLLs had their own setup. Implication: the vis API must allow one DLL to expose several modules, and data feeds must be tested with multi-module DLLs. Tag: S08.
4. **gen_bigclock2, 1.99.54**: the same plugin DLL can be loaded twice as two windows. Implication: the plugin loader must support multiple instances of one DLL, each with its own state. Tag: S09, S11.
5. **in_* hang on close and handle leaks (in_mp4, in_mp3, in_midi), 1.99.3 (Jan 2024)**: input plugins held files open or hung on unload, especially with older DSPs. Implication: unload and close ordering is a common failure point, so Rustamp needs a per-plugin close timeout and a clear unload order. Tag: S09.
6. **Input plugin deadlocks on metadata reset (FLAC), 1.99.22 (Sep 2024)**: metadata reset deadlocked, and a networking hang came from an early file-existence check in in_wav. Implication: metadata calls and file probes must not run while holding the playback lock, and must not block on the network. Tag: S09.
7. **in_psf / in_qsf metadata editing deadlock, 1.99.55 (Sep 2026)**: editing tags could deadlock. Implication: tag writes need their own thread and lock order, separate from decode. Tag: S09.
8. **in_usf2 metadata requests, 1.99.54**: metadata requests reopened files each time. Implication: give plugins a metadata cache keyed by path and mtime, to avoid repeated opens during library scans. Tag: S09.
9. **Input matching for compound extensions (NSFE, sc68.gz), 1.99.50 (May 2026) and 1.99.3 (Jan 2024)**: files with extra text after the extension, or double extensions, were matched to the wrong input plugin. Implication: plugins must declare their full extension patterns, and the matcher must try longest match first. Tag: S09.
10. **Plugin DLL search order and delay-loading, 1.8.0 (Sep 2022), 1.99.22 (Sep 2024), 1.99.54**: duplicate DLL names conflicted, and intermittent loads failed under multi-threaded delay-load. Implication: resolve plugin dependencies from the host folder first, and keep plugin loading single-threaded or fully locked. Tag: S09.
11. **Smart App Control named as a likely load-failure cause, 1.99.55 (Sep 2026)**: the error message now names this Windows feature. Implication: Rustamp should produce clear diagnostics for DLL load failures caused by OS policy, including the 32-bit case. Tag: S09.
12. **Out plugins unloaded live, 1.99.55**: out_burst and out_sqr crashed when unloaded before playback. The notsodirect output crashed after unload and could hang on rapid play and stop events. Implication: output unload must be safe at any time, and play and stop events must be serialised. Tag: S18.
13. **Small output buffers stop vis and plugin rendering, 1.99.50 (May 2026)**: the open call returned inconsistent values with small buffer sizes, which stopped visualisations. Implication: the output API must give stable return values, and vis should keep receiving data at small buffer sizes. Tag: S18, S08.
14. **Output start-up after crash, 1.99.22**: start-up settings were adjusted after a crash so that an output change, such as DirectSound to WASAPI, does not repeat the crash. Implication: keep a safe-mode fallback for the output device. Tag: S18.
15. **ASIO volume reset, 1.99.3**: WACUP warns before ASIO is activated because it resets the volume. Implication: any exclusive or ASIO-style output must warn the user and restore volume. Tag: S18.
16. **gen_ff hook timing and Streaming Source DSP threading, 1.99.3**: fixes were needed for the timing of gen_ff hooks and for threading in the streaming DSP. Implication: hook-style plugins depend on exact timing, so Rustamp should use explicit events. Tag: S09, S10.
17. **dsp_wc, 1.99.54 and 1.99.55**: Icecast 2.5.0 and later connections failed, mono resampling streams failed, and missing cleanup caused high CPU. Implication: DSP chains must have a defined cleanup and reset path, and resampling must handle mono input. Tag: S09, S18.
18. **Winamp 5.666 plugin compatibility, 1.99.3**: WACUP dropped the legacy mode and no longer uses the 5.666 program file, but still relies on some 5.666 plugins. Implication: decide early which 5.666-era plugins Rustamp supports, and treat the rest as out of scope. Tag: S09.
19. **Hook-based loader, 0.9.9.1316 (Nov 2016)**: a loader ran before the original executable, hooked plugin exports, and handled command-line options. Implication: Rustamp should not copy this hook model. Its host should load plugins directly. Tag: S09.
20. **Deadlock watchdog, 1.0.9**: deadlock detection tries to terminate a stuck process before falling back to Task Manager. Implication: the 32-bit plugin host needs a watchdog that can kill and restart it. Tag: S09, S10.
21. **Plugin double-hook and unload crashes, 1.0.9**: double-hooking crashed, and uninstalling input or general plugins also crashed. Implication: plugin install, uninstall and reload must be tested as first-class operations. Tag: S09.
22. **Unstable third-party plugins are disabled after install, 1.0.9**: some known-unstable plugins now need manual enabling. Implication: keep a quarantine or safe-mode default for unverified plugins. Tag: S09.
23. **Scrobbler and Discord status, 1.99.3 and 1.99.22**: a native Last.fm scrobbler (in Yule Log) disables itself when the official scrobbler is active. Seek notifications from UI interaction wrongly updated the Discord status when windows were moved. Implication: scrobblers need an exclusivity check, and seek events must be separated from UI events. Tag: S10.
24. **IPC handling, 0.9.9.1316, 1.0.9 and 1.99.3**: plugins can get the service pointer without SendMessage. Many IPC calls moved into the core, and a message ID clash between two calls was remapped. IPC_STARTPLAY is posted asynchronously to avoid a hang, the /WA_IPC value range was widened, and /SEEK= was added as a command-line option. Implication: prefer async posting across the process boundary, reserve IDs explicitly, and keep the command-line surface stable. Tag: S10.
25. **Hotkeys and global keys, 0.9.9.1316 and 1.99.3**: the hotkey plugins were rewritten with new EQ, volume and playlist keys. Global hotkeys are disabled under Wine. Implication: hotkeys should be a core command system, not a plugin. Tag: S10.
26. **Under Wine, 1.0.9, 1.8.0 and 1.99.3**: the Win7Shell and Win10Shell plugins are skipped, preference skins are disabled, the installer refuses Wine below 7.0, a stop bug in in_mp4 was fixed, and a font workaround was added for library buttons. Implication: Wine support is a list of disabled features. Rustamp should detect Wine and turn off the same things, not depend on them. Tag: S09, S11.

Extra note on embedded windows (counted in items 1, 3 and 23 above): classic skin embedded window minimum size of 250px scaled (1.99.3), vis window cleanup on failure (1.99.3), vis window creation skipped when video is disabled (1.0.9), and the Win10Shell taskbar close deadlock (1.99.3). Tag: S11.

---

## 2. Classic skin and UI quirks (18 items)

1. **Skin archives with directory entries after files, 1.99.47 (Mar 2026)**: MMD3 skins failed to load. Implication: the archive reader must not depend on entry order. Tag: S17.
2. **Nested skin archives, 1.99.3**: nested archives are now ignored when parsing. Implication: parse only the top level of a skin. Tag: S17.
3. **Skin PNG rendering and palette bitmaps, 1.99.47 and 1.99.3**: PNG rendering problems and palette bitmap display were fixed. Implication: use a PNG decoder that handles palette and transparency, and test with classic skin bitmaps. Tag: S17.
4. **Classic playlist drop position, 1.99.22**: dropping on the top or bottom of the playlist frame inserts at the start or end. Scrollbar and painting glitches were also fixed. Implication: match this drop behavior. Tag: S17.
5. **Playlist remover from first item, 1.99.55**: the remover advanced to the wrong item when starting from the first item. Implication: test remover boundary cases. Tag: S17.
6. **Playlist length display and saved paths, 1.99.3 and 1.99.54**: an option hides item lengths, and an advanced option saves absolute paths in saved playlists. Implication: offer both as settings, and default to relative paths only if the format allows it. Tag: S17.
7. **Classic seekbar double-click, 1.99.55**: fast double-clicks on the seekbar behaved oddly. Implication: treat a double-click on the seekbar as a single seek. Tag: S17.
8. **Dark-mode progress bars, 1.99.55**: progress bars needed a proper border in dark mode. Implication: derive skin colors from the theme, not hard-coded values. Tag: S17.
9. **Equaliser channel handling, 1.99.55 and 1.99.3**: EQ handling for more than two stereo channels was fixed, and band and limiter options were added to the on/off button menu. Implication: the EQ should work on any channel count, and its menu should expose the same options. Tag: S18, S17.
10. **kbps/kHz tooltip and font toggle, 1.8.0**: a tooltip shows the full value, and the font toggle updates live. Implication: small status fields need full-value tooltips. Tag: S17.
11. **Timer flashing while paused, 1.99.3**: a classic skin option stops the timer from flashing while paused. Implication: offer a pause-indicator option. Tag: S17.
12. **Font size shortcuts, 1.99.54**: Ctrl with the keypad plus, minus and enter change the playlist font size, and an ini option turns off the Ctrl+Enter reset. Implication: keep the shortcuts and the option. Tag: S17.
13. **Windowshade and smaller skinned windows, 0.9.9.1316**: classic windowshade mode is built in, and skinned windows can be smaller. Implication: plan windowshade for the classic layout. Tag: S17, S11.
14. **Video window double size and skinned frame, 1.0.9**: double size mode and a generic skinned frame option were added. Implication: the video window needs both a size mode and a frame choice. Tag: S11.
15. **Waveform seeker, 1.99.55, 1.99.47 and 1.99.22**: the seeker clears when the playlist is cleared, shows an empty state for items with no length, and cue files had load failures that were fixed. Implication: the seeker must handle missing length and cue data. Tag: S17.
16. **Local time display, 1.99.47**: time text in library views used UTC instead of local time. A later build still needed tuning. Implication: always display local time, and test around daylight-saving changes. Tag: S17.
17. **Playlist corruption during fadeout stop, 1.99.54**: the playlist window could corrupt when stopping during fadeout. Implication: stop and fade must be serialised with UI redraws. Tag: S17.
18. **Preferences windows on Wine, 1.0.9**: preference windows are not skinned under Wine. Implication: this is a Wine-only fallback, not a model for Rustamp. Tag: S17.

---

## 3. Modernization ideas for M4 ("modern underneath")

Each item has a "worth doing?" verdict and a reason.

1. **WASAPI shared and exclusive output, event-driven.** WACUP still ships DirectSound-era outputs and warns about ASIO volume resets, so WASAPI looks under-served. **Worth doing? Yes.** It is the main output change Rustamp can make without copying anything. Tag: S18.
2. **Gapless and crossfade output.** The notsodirect output had no gapless playback at 1.0.9, and later output fixes were about stability, not gapless. **Worth doing? Yes.** Gapless is a common user expectation, and Rustamp can build it into the core output path. Tag: S18.
3. **ASIO output.** WACUP has an ASIO path with a warning for volume resets. **Worth doing? Maybe.** Only if there is demand. Keep the warning and the volume restore if we do it. Tag: S18.
4. **ReplayGain.** A fix stopped some MP3 files being wrongly flagged as unsupported (1.99.55). The tag reader and the gain application should live in the core. **Worth doing? Yes.** It is a cheap win for a common feature. Tag: S18.
5. **Format breadth.** WACUP added Ogg FLAC, fragmented MP4, Opus seeking fixes, FLAC mono and surround handling, and Shorten (1.99.55 native plugin). **Worth doing? Yes for FLAC, Ogg, Opus, and MP4/AAC. Maybe for Shorten.** Prioritise the widely used formats. Tag: S09, S18.
6. **HiDPI and mixed-monitor scaling.** WACUP says full high-DPI and mixed-monitor support is still work in progress (1.0.9), and it fixed DPI sizing bugs in 1.99.22 and 1.99.54. **Worth doing? Yes.** Design per-monitor DPI awareness in from the start for the modern window layer, not as a patch later. Tag: S11, S17.
7. **Metadata: one-call fetch with per-value fallback, local DB reads, and changed-file refresh.** WACUP fetches metadata in one call where possible (1.99.3), reads from the local library database when available (1.99.54), and can refresh only changed files (1.99.3). **Worth doing? Yes.** This keeps large libraries fast. Tag: S09.
8. **Library ingress speed.** A library ingress check dropped from about 60 seconds to a few seconds for 30K items (1.99.3). **Worth doing? Yes.** Use a cached index and check mtime before reparsing. Tag: S09.
9. **Streaming: ICY metadata and native MP3/AAC stream playback.** Native MP3 and AAC stream playback was added to the URL input (1.99.22), and the server name comes from the icy-notice2 header (1.99.54). **Worth doing? Yes for ICY metadata and native stream playback.** Skip any dependency on an external downloader for the first milestone. Tag: S09, S18.
10. **yt-dlp-based URL resolution.** WACUP resolves YouTube and similar links with yt-dlp, which can take 3 to 5 seconds (1.99.3, 0.9.9.1316). **Worth doing? No for M4.** It adds a heavy external dependency with a moving target, and the legal and maintenance cost is high. Revisit later as an optional plugin.
11. **Network timeouts and libcurl setup.** Longer timeouts and a revised libcurl connection setup were added for slow links (1.99.54). **Worth doing? Yes.** Use one modern HTTP client with configurable timeouts from day one. Tag: S18.
12. **Crash reporting with a module list and last-crash summary.** WACUP now lists all loaded modules (1.8.0) and shows a summary of the last crash (1.99.3). **Worth doing? Yes.** This matters most for the 32-bit plugin host. Tag: S09.
13. **Performance: in-memory INI cache and fewer string-length calls.** The INI cache and the string-length reductions were both done for speed (0.9.9.1316, 1.99.54). Image resize went from about 220 ms to 60 ms on a large test image (1.99.3). **Worth doing? Yes for the caches, and yes to measuring first.** Use a profiler before optimising. Tag: S17.
14. **Duplicate and missing item logging in Media Library.** Removals are logged to report files (1.99.55). **Worth doing? Maybe.** Useful for trust, but low priority for M4. Tag: S09.
15. **Discord rich presence with an album-art toggle.** A toggle stops album art uploads, and streams can show as a button (1.8.0, 1.99.22). **Worth doing? Maybe.** Privacy controls must be on by default if we build it. Tag: S10.

---

## 4. Top 15 takeaways for the Rustamp team (ranked)

1. **Keep plugins out of the core process.** Crashes from AVS, MilkDrop presets, in_* unloads and deadlock-prone metadata calls recur across many versions. The 32-bit host with a watchdog that can kill and restart it (item 1.20) is the right call. Tag: S09.
2. **Make unload, close and reload safe at any time.** Out and in plugin unload crashes, close hangs and handle leaks show up in almost every year of the changelog. Make close ordering and per-plugin timeouts part of the ABI spec. Tag: S09, S18.
3. **Serialise metadata and tag I/O away from playback.** Several deadlocks came from metadata calls holding shared locks. Give metadata its own thread and lock order. Tag: S09.
4. **Define IPC across the process boundary with async posts and reserved IDs.** Message ID clashes, sync calls that hang, and a start-play hang all point to this. Tag: S10.
5. **Treat embedded windows as first-class.** Frame scaling, minimum sizes, vis window cleanup and video window modes all broke at some point. Specify sizes and cleanup for the embed contract. Tag: S11.
6. **Plan for DPI from the start.** WACUP's DPI and mixed-monitor work is still unfinished after years. Per-monitor DPI in the window layer is a day-one requirement. Tag: S11, S17.
7. **Build a WASAPI output that supports gapless.** This is the clearest gap in WACUP and the most visible modernization win. Tag: S18.
8. **Keep the vis API multi-module and crash-tolerant.** Multi-module vis DLLs, data regressions and small-buffer issues are all recurring. Tag: S08.
9. **Give plugins a metadata cache and fast library ingress.** Caching cut library checks from minutes to seconds in WACUP. Tag: S09.
10. **Treat Wine as a set of disabled features, not a target.** Keep a Wine detection path that turns off the same things WACUP turns off. Tag: S09, S11.
11. **Be explicit about the 5.666 plugin boundary.** WACUP still depends on some 5.666 plugins. Decide early what Rustamp supports and document it. Tag: S09.
12. **Ship an input matcher that handles compound extensions.** NSFE, sc68.gz and similar formats broke matching more than once. Tag: S09.
13. **Add ReplayGain in the core decode path.** A bug wrongly flagged MP3 as unsupported, so the feature needs its own tests. Tag: S18.
14. **Put crash reports and a module list in the first release.** WACUP's crash-reporting improvements help triage plugin crashes. Tag: S09.
15. **Keep skins robust to archive and bitmap quirks.** Directory order, nested archives and palette PNGs each caused load failures. Tag: S17.

---

## Item counts

- Section 1: 26 items (plus an embedded-window note).
- Section 2: 18 items.
- Section 3: 15 items, each with a verdict.
- Section 4: 15 items.
