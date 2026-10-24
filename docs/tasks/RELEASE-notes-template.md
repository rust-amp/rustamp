# Rustamp {release_id} — Release Notes

> **Notice:** Rustamp is an independent, clean-room reimplementation of the classic Winamp 2 interface in safe Rust. It is not affiliated with, endorsed by, or derived from Nullsoft, AOL, Radionomy, or modern Winamp owners.

---

## 1. Release Identity & Artifacts

- **Product Version:** `{product_version}`
- **Source Commit:** `{commit}` (`{commit_short}`)
- **Release Identifier:** `{release_id}`
- **IPC Protocol Version:** {protocol_version}
- **Config Schema Version:** {config_version}
- **Build Profile:** `{profile}`
- **Runtime Policy:** Statically linked Microsoft MSVC CRT (`-C target-feature=+crt-static`). No Microsoft Visual C++ Redistributable is required.

### Downloads

| File | Description | SHA-256 Checksum |
|---|---|---|
| `{release_id}.zip` | Portable release archive containing player, background hosts, licenses, and guides | See `{release_id}.zip.sha256` |
| `{release_id}.zip.sha256` | SHA-256 checksum file | Formatted for `sha256sum -c` or PowerShell |

---

## 2. Integrity Verification

Before extracting or executing the archive, verify its SHA-256 checksum against the published hash:

### PowerShell (Windows):
```powershell
Get-FileHash -Path .\{release_id}.zip -Algorithm SHA256
```
Compare the output `Hash` string with the contents of `{release_id}.zip.sha256`.

### Command Prompt (`certutil`):
```cmd
certutil -hashfile {release_id}.zip SHA256
```

### Linux / macOS / MSYS2:
```bash
sha256sum -c {release_id}.zip.sha256
```

---

## 3. How to Run (Portable ZIP)

Rustamp ships as a **standalone portable folder** published in a ZIP archive.
There is **no installer, no registry modification, no background service, and no automatic startup entry**.

1. Extract `{release_id}.zip` into a folder of your choice (e.g. `C:\Tools\Rustamp` or any user directory).
2. Open PowerShell or Command Prompt in the extracted `player` directory.
3. Launch `rustamp.exe`:
   ```powershell
   .\rustamp.exe "C:\Path\To\Your\Music.mp3"
   ```
   Or launch without arguments to start empty:
   ```powershell
   .\rustamp.exe
   ```

### Command Line Options:
- `--skin <lookalike|skin_dir|skin.wsz>`: Select a visual skin. Defaults to the built-in drawn Rust theme; pass `--skin lookalike` for classic proportions or specify a `.wsz`/`.zip` classic skin file.
- `--no-audio` / `--null`: Run headless or with null audio sink (used for verification and automated tests).

### To Uninstall:
Simply delete the extracted folder.
User configuration is saved to `%APPDATA%\Rustamp\rustamp.toml` on clean exit; you may delete that file if you wish to remove all settings.

---

## 4. Important User Disclosures

### G5. Unsigned Binaries
- **Status:** The executables in this archive are **not code-signed** with a commercial Authenticode digital certificate.
- **What this means:** When you launch `rustamp.exe` for the first time, Windows Defender SmartScreen will display a blue warning box:
  > *"Windows protected your PC — Microsoft Defender SmartScreen prevented an unrecognized app from starting."*
- **How to proceed:** Click **"More info"**, then click the **"Run anyway"** button that appears.
- **Integrity verification:** Web browsers may also report that the download is unrecognized. To ensure the download has not been corrupted or tampered with, always verify the SHA-256 hash against `{release_id}.zip.sha256`.

### G6. No Automatic Updates
- **Status:** Rustamp contains **no automatic update checker, no background update service, and no telemetry**.
- **What this means:** The player will never connect to the internet to check for newer versions or download patches.
- **How to upgrade:** Upgrades are entirely manual. When a new version is released, download the new ZIP archive from https://github.com/rustamp/rustamp/releases, extract it, and run the new executable.
- **Settings preservation:** Your user configuration in `%APPDATA%\Rustamp\rustamp.toml` is preserved across manual upgrades.

---

## 5. What This Release Can and Cannot Claim

### Proven & Functional in This Release:
- **Audio Playback:** Local playback of MP3, FLAC, and WAV/RIFF audio streams via Symphonia decoders.
- **Transport Controls:** Play, pause, stop, frame-accurate seeking, and gapless track advance.
- **Built-in Visualiser:** Implemented and wired to the playing audio stream (oscilloscope and 19-band spectrum analyzer). Clears the screen immediately the moment playback stops, in every mode.
- **Shuffle & Repeat:** Implemented and driven by the audio engine, which owns the traversal permutation and cycle replanning (including previous-cycle restoration on Prev).
- **Equalizer & DSP:** 10-band Equalizer and preamp in the DSP chain (flat/unity by default).
- **System Tray & Global Hotkeys:** Complete and operational. Global hotkey chords register and drain into the command queue; the tray icon attaches to the shell, responds to left-click, double-click, and context menu, and recovers upon Explorer restarts (`TaskbarCreated`).
- **Window Focus Commands:** Keyboard focus navigation is functional: `Ctrl+Tab` (`FocusNextWindow`) cycles across open docked windows, and `Alt+W` (`ShowMainWindow`) shows, un-minimizes, and focuses the main window (including recovery after tray hide). Note: the focus cycle (`[Main, Equalizer, Playlist]`) is an own design choice rather than a reference measurement.
- **Skins & Themes:** Built-in default drawn Rust theme skin, selectable lookalike theme (`--skin lookalike`), and external classic skin archives (.wsz, .zip, and uncompressed skin directories).
- **Single Instance:** Second-instance command-line argument forwarding via Windows named pipes (`\\.\pipe\rustamp-app-cli`).
- **Configuration:** Persistent settings saved to `%APPDATA%\Rustamp\rustamp.toml` upon clean exit.
- **Process Supervision:** Supervised background helper processes (`rustamp-dechost-symphonia.exe`, `rustamp-dsphost-stages.exe`, and bundled `rustamp-plughost32.exe`).
- **Portable Release Package & PE Import Audit:** Shipped as a standalone portable archive with complete licence inventory and full text for required licences. Executables are statically verified to import only Windows system DLLs (excluding dynamic CRT DLLs), allowing `artifact_imports_verified: true` in the manifest.

### Unproven / Not Supported / Placebos:
- **Clean Machine Proof:** Although offline static PE import inspection verifies that executables import only Windows system DLLs (`artifact_imports_verified: true`), actual execution on a bare clean Windows installation without developer runtimes has not yet been executed in a clean VM checkpoint.
- **Physical Audio Hardware:** Validated in hypervisor virtual machines with null/software sinks; physical sound card latency, hardware device-loss recovery, and diverse driver compatibility are not certified.
- **Winamp Plugins:** The 32-bit helper process (`rustamp-plughost32.exe`) and byte-level approval store exist, but user-facing UI configuration is not yet implemented. Real Winamp plugins are not proven running in the shipped player. The helper process provides out-of-process isolation, not a security sandbox.
- **Visualizer Fidelity & Vis Plugins:** The built-in visualizer's response curves and ballistics are internal design choices, not measured against or matched to a reference Winamp implementation. External Winamp visualization plugins are not hosted.
- **Placebo & Skeleton Controls:** Clutter bar O/V buttons emit no commands; menu submenus are deferred skeletons; the window menu Exit item closes only the popup rather than exiting the application; EQ presets popup and preferences pages remain skeletons.

---

## 6. Licensing & Third-Party Notices

Rustamp is built under strict clean-room and open-source compliance standards:

- **Player Application:** Licensed under **GNU General Public License v3.0 or later (GPL-3.0-or-later)** with a **Section 7 Plugin Exception** (`player/PLUGIN-EXCEPTION.md`).
- **Plugin Host Helper (`rustamp-plughost32.exe`):** Dual-licensed under **MIT OR Apache-2.0** (`host/LICENSE-MIT`, `host/LICENSE-APACHE`). Strictly isolated from copyleft obligations; its normal dependency closure consists of 35 permissive packages only.
- **Source Code Offer (GPL-3.0 §6):** Complete corresponding source code for the player and all bundled components is available at:
  - Repository: https://github.com/rustamp/rustamp
  - Release Tag: https://github.com/rustamp/rustamp/releases/tag/v{product_version}
- **Symphonia (MPL-2.0):** Audio demuxers and decoders are licensed under Mozilla Public License 2.0. The full 373-line text of MPL-2.0 is embedded in `player/THIRD_PARTY.md`. Source code is accessible at https://github.com/pdeljanov/Symphonia.
- **Apache-2.0:** Shipped dependencies licensed under Apache-2.0 carry the complete license text embedded in `player/THIRD_PARTY.md` (and beside the host in `host/LICENSE-APACHE`).
- **Webamp Classic Skin Tables:** Classic skin layout and palette tables are derived from Webamp under the MIT License (Jordan Eldredge).
- **Microsoft C Runtime (MSVC CRT):** Statically linked under the Microsoft Visual Studio Software License Terms (REDIST.TXT).
- **Complete Inventory:** Complete license texts and an inventory of all 302 crate dependencies are provided in `THIRD_PARTY.md`, `player/THIRD_PARTY.md`, and `host/THIRD_PARTY.md`.
