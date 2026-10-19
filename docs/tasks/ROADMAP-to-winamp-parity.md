# Roadmap to Winamp 2.95 Parity

**Goal:** "looks and feels like Winamp 2.95, modern backend"  
**Date:** 2026-10-19  
**Current State:** M0 complete, M1 nearly done, BETA-10 done

---

## Status Overview

### ✅ What Works (M0 + M1 + Partial M2)

**Core Infrastructure:**
- ✅ Audio engine: decode → DSP → output (gapless, seek, volume/balance)
- ✅ Decoders: MP3, FLAC, Vorbis, WAV, AAC/ALAC (symphonia)
- ✅ Output: WASAPI shared mode, cpal fallback, null output
- ✅ DSP: 10-band EQ + preamp (own design), decode chain
- ✅ Playlist model: entries, shuffle, repeat (engine-owned)
- ✅ Configuration: read/write TOML, honors settings
- ✅ Visualization: built-in spectrum + oscilloscope (own design)

**UI - Main Window:**
- ✅ Skin loading: .wsz/zip/folder, BMP all depths, fallbacks
- ✅ Main window: layout, rendering, hit-testing
- ✅ Transport: play/pause/stop/prev/next, position slider
- ✅ Volume + Balance sliders
- ✅ Marquee scrolling, time display (elapsed/remaining)
- ✅ Window buttons: minimize/shade/close
- ✅ Clutterbar: A (always on top), I (file info), D (double-size)
- ✅ Clutterbar O/V: **implemented** (opens Options/Visualization menus)
- ✅ Double-size, windowshade
- ✅ **Title bar window-move fix** (2026-10-19)

**UI - Other Windows:**
- ✅ Equalizer window: sliders, ON/AUTO, graph, presets popup
- ✅ Playlist window: scrolling, selection, visibility toggle

**UI - Dialogs:**
- ✅ Open Files dialog (typed path, dropped file, Enter confirms)
- ✅ Jump to Time dialog
- ✅ Load Playlist dialog (wired, Enter loads)
- ✅ Preferences dialog: 5 plugin pages, native Windows dialog

**Platform:**
- ✅ DPI awareness (per-monitor V2)
- ✅ Single instance + CLI forwarding
- ✅ Tray icon + context menu
- ✅ Global hotkeys (6 default Ctrl+Alt chords)
- ✅ Window focus navigation (Ctrl+Tab, Alt+W)

**Plugins/Hosting:**
- ✅ Plugin ABI types (S09)
- ✅ Plugin loader with approval store (snapshot + preflight)
- ✅ Plugin host binary (i686)
- ✅ Decoder resident (rustamp-dechost)
- ✅ DSP resident (rustamp-dsphost-stages)
- ✅ EQ presets: 9 standard presets
- ✅ Plugin UI selection: 5 pages, approval-gated, persistence

**Packaging:**
- ✅ Self-contained runtime (static CRT)
- ✅ Package tool with manifests + license inventory
- ✅ QUICKSTART generation

**Skins:**
- ✅ Built-in default: Rust theme (33 files, 19 sheets)
- ✅ Look-alike skin selectable

---

## ❌ What's Missing for Winamp 2.95 Parity

### Critical (M2 - "It feels like Winamp")

#### 1. **Menus** (Blocking: S04 needed)
- ❌ Main menu: 19 rows, 4 separators (structure exists, actions incomplete)
  - ❌ File > Open File(s) - needs native OS picker
  - ❌ File > Open Directory - needs picker
  - ❌ File > Open Location (URL) - exists but URLs not supported by decoder
  - ❌ Play > Jump to File - exists, needs implementation
  - ❌ Options > **EQ presets menu is not clickable** (S04 needed)
  - ❌ Options > Skin Browser - needs implementation
  - ✅ Options > Preferences - works (5 plugin pages)
  - ✅ Visualization submenu - exists, 4 rows
- ❌ Right-click menus - incomplete
- ❌ Playlist menus (File/Sort/Misc/List/Selection)

#### 2. **File Operations**
- ❌ Native OS file picker (blocked by Win32 binding decision)
- ❌ Drag-and-drop files from Explorer → playlist (not implemented at all)
- ❌ Add Directory: recursive scan or single-level? (needs S13 spec)
- ❌ Remove selected entries from playlist
- ❌ Clear playlist
- ❌ Save playlist (M3U/PLS/XSPF)

#### 3. **Playlist Editor** (Partially done)
- ✅ Scrolling, selection, visibility
- ❌ Drag-reorder entries
- ❌ Jump to File search
- ❌ Sort menu (needs S06)
- ❌ Misc/List menus
- ❌ Title formatting (needs S14)
- ❌ Playlist file format detection (M3U vs PLS vs XSPF)

#### 4. **Docking & Snapping** (S05 needed)
- ❌ Window docking (main + EQ + playlist)
- ❌ Snap-to-screen edges
- ❌ Group moves
- ❌ Docking state persistence

#### 5. **Shortcuts** (Partial - S03)
- ✅ Main window shortcuts: 62 rows (many implemented)
- ❌ Playlist shortcuts: S03 section 2 (not filled)
- ❌ Equalizer shortcuts: S03 section 3 (not filled)
- ❌ Global shortcuts beyond the 6 defaults

#### 6. **Keyboard Navigation**
- ❌ Up/Down in playlist (exists but needs testing)
- ❌ Home/End in playlist
- ❌ Page Up/Down in playlist
- ❌ Enter to play selected entry
- ❌ Delete to remove entry

---

### Important (M2/M3 - Polish)

#### 7. **Classic Visualizations** (S08 needed)
- ✅ Built-in spectrum/oscilloscope (own design, not reference-matched)
- ❌ S08 measurement: exact frequency response, decay ballistics
- ❌ Classic "Winamp style" analyzer modes
- ❌ Vis plugin hosting (in_openmpt loads, vis_ plugins not tested)

#### 8. **Plugin System** (M3)
- ✅ Plugin loader + approval
- ✅ i686 host binary
- ❌ in_ plugins (input): in_openmpt tested, others not
- ❌ out_ plugins (output): not implemented
- ❌ vis_ plugins (visualization): MilkDrop 2, AVS, Geiss not tested
- ❌ dsp_ plugins: not implemented
- ❌ gen_ plugins (general): not implemented
- ❌ Plugin crash isolation/restart
- ❌ Plugin configuration UI from player
- ❌ Embed window frames (gen.bmp)

#### 9. **Equalizer** (Partial)
- ✅ 10-band EQ + preamp, sliders, ON/AUTO
- ✅ 9 standard presets popup
- ❌ **EQ presets menu not clickable** (needs S04 menu window work)
- ❌ Custom presets: save/load/delete
- ❌ .eqf file format (S07)
- ❌ Auto-load per track (S07 item 13)
- ❌ Frequency response reference matching (S07 needed)

#### 10. **Media Library** (M5 - Later)
- ❌ Library database (SQLite)
- ❌ Tag editor
- ❌ Album art
- ❌ Library view
- ❌ Query/filter
- ❌ CD playback/ripping (optional)

#### 11. **External Control** (M2)
- ❌ "Winamp v1.x" window (WM_WA_IPC, WM_COMMAND)
- ❌ External tools support (AutoHotkey, scrobblers, Rainmeter)
- ❌ Media keys integration (SMTC)
- ❌ Taskbar thumb buttons
- ❌ Jump lists
- ❌ File associations

---

### Nice-to-Have (M4+ - Modern features)

#### 12. **Advanced Audio** (M4)
- ❌ WASAPI exclusive mode
- ❌ ReplayGain (scanner + playback)
- ❌ Crossfade between tracks
- ❌ Output fades (stop/pause)
- ❌ out_disk equivalent
- ❌ Device change recovery

#### 13. **Streaming** (M4)
- ❌ HTTP/Icecast/SHOUTcast
- ❌ ICY metadata (station names, track info)
- ❌ M3U8/XSPF playlist formats

#### 14. **Native MilkDrop** (M4)
- ❌ Butterchurn port to Rust
- ❌ Preset browser
- ❌ EEL2 expression engine
- ❌ Warp/comp shaders

#### 15. **Additional Codecs** (M4)
- ❌ Opus (needs libopus FFI)
- ❌ DeaDBeeF-derived decoders:
  - ❌ libopenmpt (MOD/XM/IT/S3M)
  - ❌ game-music-emu (NSF/SPC/VGM)
  - ❌ sidplayfp (SID)
  - ❌ WavPack
  - ❌ APE (Monkey's Audio)
  - ❌ MPC (Musepack)

#### 16. **Configuration** (M2)
- ✅ Read/write TOML
- ❌ winamp.ini import
- ❌ plugin.ini emulation

#### 17. **Installer** (Release)
- ❌ NSIS installer
- ❌ Code signing
- ❌ Auto-update mechanism (decided: NO for 0.1.0)

#### 18. **Cross-Platform** (M6 - Future)
- ❌ Linux backend
- ❌ macOS backend
- ❌ Plugin host under Wine

---

## Specs Still Needed (Dirty Room)

**High Priority:**
- ❌ S04: Menu trees, labels, order, checked states, EQ presets menu behavior
- ❌ S05: Docking/snapping rules, distances, group moves
- ❌ S06: Playlist behaviors (drag-reorder, sort semantics, Jump-to-file search)
- ❌ S13: Drag-and-drop rules, command-line switches, single-instance
- ❌ S14: Title formatting (%1–%7)
- ❌ S15: Shuffle algorithm details

**Medium Priority:**
- ❌ S07 refinements: frequency response, .eqf format, auto-load
- ❌ S08: Vis computation exact specs (bins, frequencies, decay)
- ❌ S10: Full WM_WA_IPC/WM_COMMAND catalog for plugin compatibility
- ❌ S11: Embed window API, gen.bmp layout
- ❌ S12: winamp.ini keys, plugin.ini conventions

**Lower Priority:**
- ❌ S16: Preferences pages (beyond current 5 plugin pages)
- ❌ S18: Output fades/prebuffering
- ❌ S19: Streaming/ICY display
- ❌ S20: Media Library (M5)

---

## Implementation Priority (Next Steps)

### **Phase 1: Make it Usable (High Priority)**

1. **Native File Picker** (Critical blocker)
   - Decision: Use Win32 `IFileOpenDialog` or stay with egui?
   - Blocks: Open File(s), Open Directory, drag-and-drop

2. **Drag-and-Drop** (User-reported missing feature)
   - Implement `winit` DragEnter/DragDrop events
   - Add `Command::AddFiles` to core
   - Test: drop single file, multiple files, folder

3. **Playlist Operations** (Critical for usability)
   - Remove selected entries (Del key)
   - Clear playlist
   - Save playlist (M3U/PLS)
   - Add directory (recursive scan decision needed)

4. **Menu Actions** (S04 dependent)
   - Implement remaining File menu actions
   - Make EQ presets menu clickable and functional
   - Skin browser
   - Jump to File

5. **Playlist Keyboard** (S03 section 2)
   - Up/Down/Home/End/PageUp/PageDown
   - Enter to play
   - Delete to remove
   - Ctrl+A select all

### **Phase 2: Make it Feel Right (Medium Priority)**

6. **Docking & Snapping** (S05 dependent)
   - Measure distances, snap tolerance
   - Implement docking logic
   - Group moves
   - Persistence

7. **Title Formatting** (S14 dependent)
   - %1–%7 variables
   - Playlist display
   - Window title

8. **Shuffle Algorithm** (S15 dependent)
   - Match Winamp 2.95 behavior
   - Seed, invalidation rules

9. **Playlist Enhancements**
   - Drag-reorder
   - Sort menu
   - Jump to File search

10. **EQ Enhancements**
    - Custom preset save/load/delete
    - .eqf file format
    - Auto-load per track

### **Phase 3: Plugin Compatibility (M3)**

11. **Plugin Hosting**
    - Test vis_ plugins: MilkDrop 2, AVS, Geiss
    - Test in_ plugins beyond in_openmpt
    - Implement out_ plugins
    - Implement dsp_ plugins
    - Implement gen_ plugins
    - Crash isolation/restart

12. **External Control Window**
    - WM_WA_IPC implementation
    - WM_COMMAND catalog
    - Test with real external tools

### **Phase 4: Modern Features (M4)**

13. **Advanced Audio**
    - WASAPI exclusive mode
    - ReplayGain
    - Crossfade
    - Device change recovery

14. **Streaming**
    - HTTP/Icecast support
    - ICY metadata

15. **Native MilkDrop**
    - Butterchurn port
    - Preset browser

---

## Testing & Verification Gaps

**Never Tested:**
- ❌ Packaged release run in VM (exists, never launched)
- ❌ Resident tests (`resident.rs`, `flush_contract.rs`, M17 updates) - written, never executed
- ❌ Plugin loader i686 tests - written, never executed
- ❌ Clean machine test (no dev runtimes) - no bare OS baseline

**Partially Tested:**
- ⚠️ Clutter O/V popups - code works, never manually tested in VM GUI
- ⚠️ Plugin UI pages - code works, never manually tested in VM GUI
- ⚠️ EQ presets popup - code works, not clickable (needs S04)
- ⚠️ Load Playlist Enter - wired, never VM-verified

---

## Decisions Needed

1. **File Picker:** Native Win32 `IFileOpenDialog` or keep egui dialog?
2. **Drag-and-Drop:** Spec needed - where can you drop? Append or replace?
3. **Add Directory:** Recursive scan or single-level?
4. **URL Support:** Which schemes? http/https/ftp/mms?
5. **Plugin Approval:** Auto-approve signed plugins? Manual only?
6. **Installer:** NSIS or MSI? Code signing provider?
7. **Git Remote:** Where to host? GitHub/GitLab/self-hosted?

---

## Estimated Effort (Rough)

| Phase | Items | Effort |
|-------|-------|--------|
| Phase 1: Usable | 5 items | ~2-3 weeks |
| Phase 2: Feel Right | 5 items | ~2-4 weeks |
| Phase 3: Plugins | 2 items | ~3-5 weeks |
| Phase 4: Modern | 3 items | ~4-6 weeks |
| **Total** | **15 major items** | **~11-18 weeks** |

This assumes:
- Specs (S04-S15) arrive incrementally
- No major architecture changes
- Parallel work on independent items
- Testing in VM as we go

---

## Next Immediate Actions (Today)

1. ✅ **Fix Title Bar Window-Move** - DONE (2026-10-19)
2. ⏭️ **Decide:** Native file picker or egui?
3. ⏭️ **Implement Drag-and-Drop** (1-2 days, needs decision on behavior)
4. ⏭️ **Test Packaged Release in VM** (critical verification gap)
5. ⏭️ **Playlist Remove/Clear** (1 day, unblocked)

---

## Owner Decisions Needed Now

**A. File Picker:**
- Option 1: Implement native Win32 `IFileOpenDialog` (more work, native UX)
- Option 2: Keep current egui dialog + improve it (less work, non-native UX)

**B. Drag-and-Drop Spec:**
- Where can files be dropped? (main window? playlist? both?)
- Behavior: append to playlist or replace?
- Folder drop: recursive scan?

**C. Priority:**
- Which Phase 1 items are most critical?
- Should we focus on usability (Phase 1) or plugin compatibility (Phase 3)?

**D. Release Timeline:**
- Target date for 0.1.0?
- Should we ship Phase 1 only, or wait for Phase 2?
