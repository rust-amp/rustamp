# Rustamp 1.2.0

Rustamp is a Windows music player with a classic skin interface. This release
turns the classic plug-in support into something you can rely on, adds a
MilkDrop window of our own, and ships an installer next to the portable zip.

## Classic plug-ins

- Input, output, visualization, DSP and general purpose plug-ins still run in a
  separate 32-bit helper process, so a crashing plug-in cannot take the player
  down. What changed is what they get:
  - The audio helpers are told which track plays, and a pause reaches the output
    plug-in, not only the engine.
  - An output plug-in's own clock now sets how much of a track the engine counts
    as played, so the end of a track is no longer cut off when the plug-in still
    holds audio.
  - A DSP plug-in's configuration dialog opens in the instance that actually
    processes the audio, and a DSP library's second and later modules can be
    chosen and are kept in the configuration.
  - A visualization plug-in's window is fitted to its frame, and it gets the
    player's skinned frame (`gen.bmp`) instead of the system caption.
  - A file named on the command line, `/ADD`, and the plug-in's own requests for
    the current track all reach the host the same way.
- Verified in the test VM against the real Winamp 5.66 plug-ins:
  <!-- VM: list from the acceptance run, filled in after the run -->

## MilkDrop

`rustamp-milkdrop` renders MilkDrop presets on the `lactea` engine and has its
own window (Visualization menu, `Alt+D`) with next/previous/random, timed and
random preset changes, folder textures, and a CPU-readable frame for the tests.

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

## Known limitations

- The binaries are unsigned; Windows SmartScreen may warn on first start.
- There is no auto-update.
- Real-hardware audio and GPU rendering are not verified; the acceptance runs
  use the test VM's virtual cable and software adapters.
- <!-- VM: remaining measured differences, filled in after the acceptance run -->
