# Continuous-run session log — 2026-10-19

This session ran the lane cycle continuously: start a lane in its own worktree,
verify it, merge it, delete the worktree, refill. Every merged lane was verified
by the orchestrator (`fmt`, `clippy`, the relevant tests, and a workspace check
after every merge), not by trusting the lane's report.

## Merged (in order)

| Lane | What it delivered |
|---|---|
| — (baseline `fff9225`) | phase 1: playlist keyboard, drag-drop, save playlist, menu actions, save-playlist UI |
| `wave127` | playlist remove-dead-files (40222) + clear (40214), RFC 0018 |
| `wave129` | jump-to-file row overflow fix, count line, stale selection index, tests |
| `wave130` | **docking integration** (the `docking.rs` model was unused; now wired, with snapping + persistence). Retried on GPT-6.1-SOL after the first attempt produced alias spam |
| `wave131` | EQ presets `.eqf` (merged with the baseline) |
| `wave132` | playlist format detection: the `sniff` window was 4 KiB and mis-detected PLS with a long comment; import API added |
| `wave133` | ReplayGain 1.0 scanner (`rustamp-meta`) |
| `wave134` | RFC 0012 device-error contract pinned with tests + VM-only marker |
| `wave135` | mediahost PCM transfer: truncated transfer is a loud failure, stub facade removed |
| `wave136` | engine ReplayGain stage with clip protection |
| `wave138` | S15 shuffle/repeat order model (`rustamp-playlist`) |
| `wave140` | base-2.91 skin acceptance suite + sprite/font/shade fixes; regression (antialiased playlist rows) caught and fixed |
| `wave141` | `audio.replaygain` config wired to the engine |
| `wave142` | WASAPI exclusive mode + shared-mode downgrade report |
| `wave143` | the app keeps the loaded playlist format and uses it as the save default |
| `wave144` | classic analyser modes (peak hold, triggered scope) in `rustamp-vis` |
| `wave145` | SQLite media-library index (scan/query, four defects in the prior attempt fixed) |
| `wave146` | `out_` plugin support |
| `wave147` | atomic tag writes; unknown items preserved per format family |
| `wave148` | the UI uses the shared `rustamp-vis` analyser; duplicate maths deleted |
| `wave149` | external-control surface (`WM_WA_IPC`/`WM_COMMAND`, `Winamp v1.x`), ID table |
| `wave150` | Preferences: ReplayGain + output mode, RFC 0019 |
| `wave151` | playlist drag-to-reorder with insert line and identity-preserving moves |
| `wave152` | one shared `PlaybackOrder` for the engine and the decode thread (−642 lines of duplicate) |
| `wave153` | library multi-root, unicode freetext search, sort + pagination |
| `wave154` | native directory picker (`IFileOpenDialog` + `FOS_PICKFOLDERS`) |
| `wave155` | embedded artwork reader (APIC/FLAC/Vorbis/covr) |
| `wave156` | `audio.output_mode` actually applied, with downgrade notes and honest persistence |
| `wave157` | `vis_` plugin support (MilkDrop/AVS/Geiss class) |
| `wave158` | "Open directory" end to end (40187, Shift+L/Insert), recursive as an own choice |
| `wave159` | playlist MISC menu (40292/40293) per S04 §7 |
| `wave160` | `dsp_` plugin support — all five classic kinds now load |
| `wave161` | device-invalidation recovery decision + loud report (RFC 0012 fail-to-stop) |
| `wave162` | EQ auto-load per track (`eq-autoload.toml`), own-choice rules documented |
| `wave163` | playlist windowshade toggle (was a no-op), reflow + persistence |
| `wave164` | skin cursor slots + hit-region mapping |
| `wave165` | plugin fault attribution + bounded crash restart policy |
| `wave166` | skin cursors applied (`LoadCursorFromFileW`; `CreateIconFromResourceEx` measured to fail on in-memory bytes) |
| `wave167` | RFC 0007 cross-process wake proof with latency bound and no-busy-spin |
| `wave168` | media-library view model + column layout |
| `wave169` | library tag writes update the row from the read-back file |
| `wave170` | window-region mask builder from `region.txt` |
| `wave171` | media-library renderer |
| `wave172` | region mask applied via `SetWindowRgn` |
| `wave173` | library search field + scrollbar chrome |
| `wave174` | cancellable library scan with progress events and batch commits |
| `wave175` | mediahost engine adapter gaps: validated spec, engine-owned liveness, two-process proof |
| `wave176` | equal-power fade stage; `StopWithFadeout` now really fades |
| `wave177` | media-library window in the app (modern shell, index, view, input, scan) |
| `wave178` | bounded artwork cache (decoded thumbnails, `image`, MIT/Apache) |
| `wave179` | library cover column with a borrowed RGBA view (UI stays free of the library crate) |
| `wave180` | a ring reset now signals the wake so blocked peers see the new generation (M14) |
| `wave181` | resident restart policy: retry once, then fail the track (RFC 0006 §8) |
| `wave182` | Preferences: media-library roots (add/remove, DB as the source of truth) |
| `wave183` | `pledit.txt` `Font=` honoured (family/size/style), 1-bit invariant kept |
| `wave184` | test doubles refuse the ring direction production refuses (M14), mutation-proven |
| `wave185` | plugin config routing for every kind; `ConfigureVisPlugin` finally reaches the host |
| `wave186` | artwork covers borrowed into the library renderer |
| `wave187` | `frame_size` error parity in the single offer check, mutation-proven |
| `wave188` | `viscolor.txt` slot mapping per S17: the gradient orientation was inverted, all five scope slots now used |
| `wave189` | `in_` plugin host driver (play/pause/unpause/stop, clock) + acceptance fixture |
| `wave190` | a truncated stream is a loud decode failure, not EOS (RIFF/WAVE oracle; limits named for the rest) |
| `wave191` | the app sends `ConfigureVisPlugin` over the control channel; a missing host is reported |
| `wave192` | input→output handoff: `out_module` + volume/pan before `play`, `0x402` end-of-track |
| `wave193` | RFC 0020 draft: a generic "configure plugin N" wire message (not accepted) |
| `wave194` | EQ windowshade: shade-aware input (the unshaded layout used to answer), toggle, persistence |
| `wave195` | real `in_`/`out_` handoff acceptance fixture: the trace proves both sides |
| `wave196` | kill-the-host acceptance: whole frames then a sticky loud error, never a torn frame or silent EOS |
| `wave197` | title-bar double-click toggles windowshade on all three windows |
| `wave198` | `vis_` render loop: zeroed rows without a frame, module-paced, start/stop ordering |
| `wave199` | album ReplayGain grouping over pooled histograms |
| `wave200` | DSP-path kill tests: round trip, sticky error, write-direction wake |
| `wave201` | S03 §2 playlist shortcut table as data (83 bound, 5 explicitly unimplemented) |
| `wave202` | registry start/stop/step for the vis loop; `PluginRegistry` proven not `Send` |
| `wave203` | sidecar cover art fallback with stat-based invalidation |
| `wave204` | DSP write means *delivered*: a delivery fence closes the silent-frame gap |
| `wave205` | real BS.1770-4 integrated loudness for the R128 tags (first attempt sent back: 3–6 dB stereo error) |
| `wave206` | property tests for the S15 order model (no model bug found) |
| `wave207` | the app reads `PluginLoaded`/`PluginError` into a host-scoped inventory |
| `wave208` | order follow-ups: the re-anchor doc lie, unused `history` removed, `skip_by` clamped |
| `wave209` | enabling a plugin asks the host to load it; the host is spawned with its approval store |
| `wave210` | MP3 Xing/Info header as the truncation oracle (frames **and** bytes must be short) |
| `wave211` | pure device selection from the config, one matching rule for resolver and backends |
| `wave212` | enabled plugins load when a host attaches, once per host generation |
| `wave213` | runtime playlist font size (`Ctrl+Num+/-`), row pitch follows the font |
| `wave214` | exact AIFF truncation oracle (`SSND` header correction + two-condition rule) |
| `wave215` | the app honours `audio.device`, pins the named output, serialises it honestly |
| `wave216` | FLAC has no exact truncation oracle — proven with measurements, limit pinned |
| `wave217` | the ReplayGain filter is designed at the source rate (44.1 kHz was up to 4.6 dB off at 20 kHz) |
| `wave218` | "Generate HTML playlist" (40292) with escaping and a native save dialog |
| `wave220` | the library removes rows whose files are gone, keeps unreadable ones and reports them |
| `wave219` | the MP4 oracle exists (`mdhd` + `mdat`); a cut file is named at open instead of a bare I/O error |
| `wave221` | "Read extended info on selection" (40293) with tags or a stated reason (own choices) |
| `wave223` | a FLAC without `STREAMINFO` and a cut metadata chain are named, not "end of stream" |
| `wave224` | the DSP session's terminal diagnosis is sticky now (product fix found by property tests) |
| `wave225` | the library window removes the missing entries (`Ctrl+M`): the rows whose file is gone are dropped, the unreadable ones are kept and reported, and a running scan refuses the action |
| `wave227` | `last_scanned_at` per root with an idempotent schema migration |

Also this session: the post-merge check was a **false green** because it ran
`cargo check --workspace` without `--all-targets`; three test call sites were
broken for a whole wave. The rule in `AGENTS.md` now demands
`--all-targets --all-features`.

In flight at the time of writing: `wave226` (the `Ctrl+E`/40255 edit-entry
dialog model), `wave228` (the Ogg EOS-page oracle, or the proven limit),
`wave229` (scan progress in the library window).

## Rules added to `AGENTS.md` during the session

1. **One worktree per subagent, created before it starts**; delete it after the merge.
2. **Garbage output → restart the task on a stronger model** (alias spam, dead code).
3. **A lane must leave the workspace compiling** — and the orchestrator runs
   `cargo check --workspace` after **every** merge.
4. **Headless tests run on the host**; only window/GUI tests need the VM.

## Open points

- **Specs that do not exist yet (dirty room):** S08 (vis measurement), S11 (embed
  frames / `gen.bmp`), S12 (`winamp.ini` keys), S13 (drag-drop rules), S14 (title
  formatting), S16 (more preferences pages), S18 (fades), S19 (streaming), S20
  (media library).
- **S04 does not export the playlist menu trees** (File/Sort/List/Selection) — only
  the MISC menu is settled; no provisional labels were invented.
- **RFC 0020** is needed for a generic "configure plugin N" wire message
  (`PROTOCOL_VERSION` 4 → 5); the host-side API is ready (`wave185`).
- **VM verification** of everything window-shaped is still outstanding: the
  library window, the folder picker, the prefs pages, window shaping, skin cursors,
  the plugin acceptance tests, the exclusive output, the device-loss test.
- **The `resident_to_inprocess_source_change_is_gapless` flake** is pre-existing
  and load-dependent; it passes in isolation. `--test-threads=1` helps but is not
  a guarantee under heavy parallel builds.
- The `SelectedBG` colour question from the base skin (`#0000C6` vs the built-in
  `#0000FF`) still needs an owner decision: adopting it requires re-blessing two
  playlist goldens.
