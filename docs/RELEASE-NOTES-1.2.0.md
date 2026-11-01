# Rustamp 1.2.0

Rustamp is a Windows music player with a classic skin interface. This release
turns the classic plug-in support into something you can rely on, runs MilkDrop
2.25c through it, and ships an installer next to the portable zip.

## Classic plug-ins

- Input, output, visualization, DSP and general purpose plug-ins still run in a
  separate 32-bit helper process, so a crashing plug-in cannot take the player
  down. What changed is what they get:
  - The audio helpers are told which track plays, and a pause reaches the output
    plug-in, not only the engine.
  - An output plug-in's own clock reports how much of a track it has played, so
    the end of a track is not cut off when the plug-in still holds audio. In the
    test VM `out_wave` plays a whole tone to the virtual cable, a four-second
    pause is four seconds of silence, and the tone comes back after it.
  - A DSP plug-in's configuration dialog opens in the instance that actually
    processes the audio, and a DSP library's second and later modules can be
    chosen and are kept in the configuration.
  - A visualization plug-in's window is fitted to its frame, and it gets the
    player's skinned frame (`gen.bmp`) instead of the system caption.
  - A file named on the command line, `/ADD`, and the plug-in's own requests for
    the current track all reach the host the same way.
- Verified in the test VM against the real Winamp 5.66 plug-ins: all eleven load
  and name their descriptions, and the end-to-end tests drive them through the
  player — `in_wave`, `in_flac`, `in_vorbis`, `out_wave`, `out_disk`, the
  Optilab DSP, `vis_avs` (2.82), `vis_milk2` (MilkDrop 2.25c) and
  `vis_chavs` (Cockos Happy AVS, both EEL generations) each start, draw or play
  in the real player. The table is in
  [docs/notes/real-plugin-survey.md](notes/real-plugin-survey.md).

## MilkDrop

**MilkDrop 2.25c runs inside the plug-in host** and is verified end to end in the
test VM: the player's own decoded audio feeds it, it draws a live preset, and its
window carries the player's skinned frame.

The player's **own** MilkDrop window (`rustamp-milkdrop` on the `lactea` engine,
Visualization menu and `Alt+D`) is built and its engine renders offscreen in the
tests, but it is **not part of this release**: in the test VM the player stops
its tick after the window's first paint, so the picture never arrives. The
command answers with a note that names the plug-in path instead. The evidence
and the open symptom are in [docs/HANDOFF.md](HANDOFF.md).

## Crossfade and Preferences

- A crossfade of 0 to 30 seconds between tracks, applied behind the converters
  so each track keeps its own replay gain, DSP chain and sample rate.
- Preferences grew the pages that were placeholders: a Visualization page with
  the vis mode, the output page with the player's own output as a selectable
  row, plug-in pages with an explicit switch-on and the Approve step, snap
  distance, the skin folder and the plug-in folder.

## Installer

`cargo xtask package --installer` and the CI release job build a per-user NSIS
installer (`*-setup.exe`) next to the portable zip, with a Start menu shortcut
and an uninstaller. It installs into `%LOCALAPPDATA%\Programs\Rustamp` without
administrator rights and never touches `%APPDATA%\Rustamp`. A certificate in the
environment signs every shipped EXE and the installer; without one the build
says it is unsigned.

## Fixes

- The playlist's selection band runs to the right rail (x 12..255), as the
  reference does, instead of stopping 2 px short.
- `/ADD` follows the measurement: a running player only appends, a first start
  then plays the list from its first entry.
- The equalizer window, the main menu and the transport keep their state and
  their keys in the cases the manual GUI pass found.
- Text and goldens draw a bundled public-domain face instead of an installed
  font, so the rendering tests do not depend on the machine.
- After a seek forward, a track ends where it ends instead of playing silence for
  as long as the seek skipped, and after a seek back its end is no longer cut off.
- A plug-in's own path has no `\\?\` prefix and arrives in the system's code page,
  so a plug-in in a folder with non-ASCII letters still finds its data.

## Known limitations

- The player's own MilkDrop window is built but not part of this release (see
  above and `docs/HANDOFF.md`).
- The binaries are unsigned; Windows SmartScreen may warn on first start.
- There is no auto-update.
- Real-hardware audio and GPU rendering are not verified; the acceptance runs
  use the test VM's virtual cable and software adapters.
- The installer ran silently in the test VM (install, a start of the installed
  player, uninstall); its wizard pages have not been walked through by hand.


