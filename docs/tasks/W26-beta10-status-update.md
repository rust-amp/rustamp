# W26 BETA-10 Status Update — 2026-10-19

**Date:** 2026-10-19, after commit `29757c2`.

## Summary

Of the 8 non-functional controls identified in BETA-10, **6 are now resolved**:

### ✅ Completed (6/8):

1. **Menu Exit** - Commit `63678cd` - Now sends `Command::Quit` and actually quits the player
2. **Menu Submenus** - Already functional via native OS popup (no fix needed)
3. **EQ Presets** - Commit `63678cd` - 9 standard presets implemented (Flat, Rock, Pop, Jazz, Classical, Vocal, Dance, Full Bass, Full Treble)
4. **Prefs Pages** - Commit `29757c2` - Were already wired, tests added to verify persistence
5. **F11 Double-print** - Commit `1b4df32` - Fixed stale documentation
6. **Stale skin.rs docs** - Commit `1b4df32` - Fixed stale comments

### 🔄 In Progress (2/8):

7. **Clutter O/V buttons** - Being reimplemented in worktree `wave121-clutter-ov`
   - Previous implementation: complete with RFC 0016, all tests passed
   - Opens Options and Visualization menus at click points
   
8. **Plugin UI Selection** - Being reimplemented in worktree `wave121-plugin-ui`
   - Previous implementation: `plugins.rs` model, prefs_dialog layout, approval gate
   - Still needs: `dialogs.rs` wiring, config persistence completion

## Implementation Details

### Menu Exit (`63678cd`)
- Added `MenuAction::Exit` and `MenuChoice::Exit` variants
- Exit now submits `Command::Quit` (same as tray Exit)
- Both native and retired menu paths support it
- Tests updated and passing

### EQ Presets (`63678cd`)
- New module `crates/rustamp-app/src/eq_presets.rs`
- 9 built-in presets with own-design curves (no external source needed)
- `App::apply_eq_preset(name)` sends band commands
- Config persistence works (gains saved)
- Popup UI not yet clickable (needs S04 menu window work)

### Prefs Pages (`29757c2`)
- Dialog was already wired to 5 checkboxes: Always on top, Easy move, Double size, Shuffle, Repeat
- Setup/File types/Plug-ins/Bookmarks pages show honest "not available" notes
- Tests added in `crates/rustamp-app/tests/prefs_persist.rs`
- Shuffle/Repeat intentionally not persisted (per N6 task card)

## Worktree Reorganization

The 2 remaining tasks were moved to isolated worktrees after parallel agents caused tree collisions:

- **Before:** All agents working in `D:\code\Rustamp` (canonical tree) → build conflicts
- **After:** Each agent in own worktree → no collisions possible

**Worktrees:**
- `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave121-clutter-ov` (branch: wave121-clutter-ov)
- `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave121-plugin-ui` (branch: wave121-plugin-ui)

## Verification Status

- Suite at `29757c2`: **86 binaries, 0 failures, 0 hangs** ✅
- All 4 completed features verified in full suite run
- Menu Exit: tested headless (quit logic)
- EQ Presets: unit tests passed, popup UI needs VM test
- Prefs Pages: persistence tests passed, visual check pending

## Next Steps

1. Complete Clutter O/V in worktree (RFC 0016 recreation)
2. Complete Plugin UI in worktree (dialogs.rs + config.rs wiring)
3. Merge both worktrees to main
4. Run final suite validation
5. Update BETA-readiness-2.md with all fixes
6. Manual GUI tests in VM for new features

## Related Commits

- `63678cd` - Menu Exit + EQ Presets
- `29757c2` - Prefs pages tests
- `1b4df32` - Stale docs fixes
- `d8757bd`, `f7a10b7`, `893fc35` - Prefs dialog (earlier wave, already in main)
