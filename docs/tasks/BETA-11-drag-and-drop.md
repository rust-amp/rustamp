# BETA-11: Drag-and-Drop Support

**Status:** Not started  
**Priority:** Medium (user-reported missing feature)  
**Reported:** 2026-10-19 by owner  
**Milestone:** M2 or later

## Issue

Drag-and-drop does not work when clicking and holding the window title bar (or anywhere on the window). This is expected because **drag-and-drop is not yet implemented at all**.

### Expected Behavior (Winamp 2.95)

1. **Files from Explorer** → Main window or playlist window
   - Adds files to the current playlist
   - If dropped on main window: appends to end of list
   - If dropped on playlist: inserts at drop position

2. **URLs** → Main window or playlist window
   - Opens URL dialog or adds directly

3. **Playlist files** (.m3u, .pls, etc.) → Main window
   - Loads the playlist (replacing or appending, TBD)

### Current State

- No drag-and-drop handling exists
- `presenter.rs` only handles mouse clicks for UI widgets
- No Windows OLE Drag-and-Drop integration
- No `winit` drag-and-drop event handling

## Implementation Plan

### 1. Enable `winit` Drag-and-Drop Events

`winit` 0.30 supports drag-and-drop through `WindowEvent::DragEntered`, `DragOver`, `DragLeft`, `DragDrop`.

**Files:**
- `crates/rustamp-ui/src/presenter.rs` - add event handlers

### 2. Contract Extension

Add to `rustamp-core`:
```rust
pub enum Command {
    // ...
    AddFiles { paths: Vec<PathBuf>, play_first: bool },
    AddUrl(String),
}
```

Or use existing `OpenMedia` with a `play: bool` flag.

### 3. Platform Integration

**Windows:** `winit` uses `IDropTarget` internally, should work automatically  
**Linux/macOS:** Future milestone (M6)

### 4. Acceptance Tests

- [ ] Drop single file → adds to playlist
- [ ] Drop multiple files → adds all in order
- [ ] Drop folder → adds all supported files recursively (or single folder?)
- [ ] Drop onto title bar → still works (no window move conflict)
- [ ] Drop onto playlist window → inserts at position (future)
- [ ] Drop unsupported file → ignored or error message (TBD)

## Specification Needs

**Requires measurement from Winamp 2.95:**
- Drop onto main window: append or replace?
- Drop onto playlist: insert at position or append?
- Drop folder: recursive scan or single level?
- Drop multiple items: preserve order?
- Drop non-audio file: ignore silently or show error?
- Visual feedback during drag (highlight? cursor change?)

**Spec ID:** S-XX (not yet assigned)

## Blocking Issues

1. **No spec exists** - needs dirty-room measurement
2. **Playlist window not implemented** - main window only for now
3. **Folder recursion policy** - decision needed

## Workaround

Use File menu → Add file(s) or Add directory (when implemented).

## Related

- RFC 0005 §11 - File menu commands
- Task M2 - Main menu implementation
- Milestone M6 - Linux/macOS support

## Notes

- Owner tested in double-size mode
- Title bar drag might conflict with window move (needs testing)
- Consider: drop during playback - interrupt or queue?
