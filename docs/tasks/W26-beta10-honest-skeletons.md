# W26 — BETA-10: honest skeletons, decision boundary

**Status: blocked on the explicit beta presentation decision below; BETA-10 is not closed.**
**Date:** 2026-10-19. **Reviewed HEAD:** `a7dbded6be77feaa56ba4f78dae6a4697cc4cf13`.
**Write boundary:** only `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave126-beta10`.
Owned source paths: `crates/rustamp-prefs/**`, `crates/rustamp-app/**`,
`crates/rustamp-ui/**`; this card is the only changed file. Nothing under
`D:\code\Rustamp` may be written. The worktree existed and its initial
`git status --short` was empty.

## 1. Stop decision, not an implementation claim

The assignment explicitly requires stopping when the specifications do not
decide whether an unsupported control should be hidden or disabled. They do not:

- S04 §5 (`specs/S04-menus.md:304-329`) describes four active visualization
  menu rows, explicitly no greyed rows, and leaves click-through unmeasured.
- S04 §6 (`specs/S04-menus.md:340-356`) describes active Load/Save/Delete
  entries and leaves their submenus and load/save behavior open.
- S07 §12 O12 (`specs/S07-eq-dsp.md:586-589`) still leaves preset semantics
  open. S07 §1 (`:91`) describes AUTO as autoloading, not merely a stored flag.
- There is no exported S16 page/control specification in this worktree.
  `specs/README.md:26` lists it as not started. The index is not used to deny
  the existence of other exported specs: S04 and S07 themselves were read.
- BETA-10 (`docs/tasks/BETA-readiness.md:88-92`) requires exclusion or
  disabling, but does not choose their appearance or reachability per control.

**Question for the release owner/specification lane:** for each of the entries
in section 3, should the beta (a) retain an explicitly unavailable label and
disable activation, (b) hide the entry, or (c) retain activation solely to open
a visible, explicitly non-functional explanation? In particular, approve the
skinned PRESETS/AUTO/V treatment and its visible disabled artwork/overlay, not
only its input behavior. Should the existing warned Preferences frame remain
reachable via Ctrl+P/Ctrl+K/Alt+O, or should those menu entries and shortcuts
also be unavailable? Specify the keyboard and external-command response along
with the mouse/menu policy. No choice is silently made by this lane.

The native menu already has an `enabled` field, so a later approved disabled
menu fix does not inherently need a platform change. The skinned controls have
no specified disabled state; removing their release commands alone would still
leave active-looking placebo buttons. No renderer, behavior, test or golden
was changed to pretend this decision had been supplied.

## 2. Verify the original claims before acting

All line numbers below are in reviewed HEAD. Commits are attributed with
`git blame`/`git log`, not inferred from an old handoff.

| BETA-10 claim | Current finding | Source and commit |
|---|---|---|
| `rustamp-vis` is doc-only | Stale as a blanket claim: it exports a projectM module. This does not establish a working player-side classic-vis/plugin path. | `crates/rustamp-vis/src/lib.rs:7-13`, `427c3deb` |
| Engine tap retains raw waveform and leaves plugin spectrum zero | Still true, intentionally awaiting S08; zero plugin-spectrum bytes do **not** imply a blank main display. | `crates/rustamp-audio/src/vistap.rs:8-22`, original `c73d33d8`, UI distinction `a2106ed5` |
| Main renderer draws no live visualizer | Stale. The app drains actual engine frames with source sample rate, UI computes its own analyzer/scope, renderer draws it. S08 reference parity and decode-ahead synchronization remain open, not claimed. | `crates/rustamp-app/src/app.rs:835-837,853-873`; `crates/rustamp-ui/src/visualization.rs:2-22`; `crates/rustamp-ui/src/render.rs:198-203`, `a2106ed5` (later diagnostic wording `b5a7d13`) |
| Host loads no DLL and ignores LoadPlugin | Stale. Control forwards LoadPlugin, UI thread invokes registry, registry invokes approved loader. Input/DSP/general are supported kinds; this is not visualization-plugin compatibility evidence. | `crates/rustamp-plughost/src/control.rs:158-160`, `main.rs:333-339`, `registry.rs:213-236`, `9b7ef7c0`; `loader.rs:1098-1127`, `8a915631`/`2f4bf17b` (earlier loader `63619b98`) |
| Configure/toggle visualization plugin only records a note | Still true, even though the general DLL-loading claim is stale. The app never routes these actions to a vis lifecycle. | `crates/rustamp-app/src/app.rs:1641-1643`, `efd1d884` |
| Only two preferences page identities; no settings controls | Still true. Neither page offers an editable setting. | `crates/rustamp-prefs/src/lib.rs:21-22,44-60`, `4ada21f6`; `crates/rustamp-app/src/dialogs.rs:313-316` |
| Preferences draws only the page name | Stale in this narrower respect: it already also says `Settings on this page are not applied yet.` No new warning is needed merely to duplicate that fix. | `crates/rustamp-app/src/dialogs.rs:310-316`, `b6d1d5f7` |
| EQ presets only records an unimplemented note | Still true. The pending request opens no popup or preset system. | `crates/rustamp-app/src/app.rs:751-765`, `b3d01f8f` |

## 3. Exposed-control inventory and disposition

Disposition for every unsupported entry below: **left unchanged pending the
section 1 decision, not accepted as beta-ready**. Only the already warned
Preferences frame and the functioning main visualizer are explicitly
distinguished; they must not be reported as newly repaired here.

| Control / reachable path | What actually happens; evidence | Required honesty follow-up after approval |
|---|---|---|
| Main menu > Visualization > Select visualization plug-in; Ctrl+K (40191) | Opens the VisPlugin preferences frame, no selection control. `rustamp-ui/src/menu.rs:676-683` (`fc579003`); `shortcuts.rs:1029`; `rustamp-app/src/dialogs.rs:313-316` (`b6d1d5f7`). All crate paths in this table are under `crates/`. | Choose explicit unavailable label/disable/hide/explanation; retain the existing warning if reachable. |
| Main menu > Visualization > Visualization options; Alt+O (40317) | Opens VisOptions frame, no options controls. `rustamp-ui/src/menu.rs:684-690` (`fc579003`); `shortcuts.rs:1030`; same warned body. | Same decision; do not disable the working PCM visualizer itself merely because preferences are absent. |
| Main menu > Visualization > Configure visualization plug-in; Alt+K (40221) | Only app note. `rustamp-ui/src/menu.rs:691-697` (`fc579003`); `shortcuts.rs:1046`; `rustamp-app/src/app.rs:1641-1643` (`efd1d884`). | Must cease looking like a functioning config operation; cover shortcut and direct Command too. |
| Main menu > Visualization > Start/stop visualization plug-in; Ctrl+Shift+K (40192) | Only app note, no vis plugin starts/stops. `rustamp-ui/src/menu.rs:698-704` (`fc579003`); `shortcuts.rs:1047`; same app arm. | Same availability decision and all input routes. |
| Main menu > Options > Preferences; Ctrl+P (40012) | Opens default VisPlugin frame, already visibly warns settings are not applied. `rustamp-ui/src/menu.rs:593-599` (`fc579003`); `rustamp-prefs/src/lib.rs:50,67-70`; warned body above. | Decide whether warned informational shell is sufficient; no invented S16 controls. |
| Main-window clutter V | Shows pressed/hint state but release falls through without opening a menu. `rustamp-ui/src/behaviour.rs:238,480-525,633` (release limitation documented `4bf70262`); S02 §3 row 5 / S04 §5 expect a vis menu. | A mouse no-op needs visibly unavailable presentation or a clearly marked explanation, not just a missing command. |
| EQ PRESETS click; plain S while EQ focused | Queues menu request, app consumes it into a note, no popup. `rustamp-ui/src/equalizer.rs:557-558,587-589`; `rustamp-app/src/app.rs:1258-1265,751-765` (`b3d01f8f`). | Decide skinned button appearance and keyboard response; do not implement Load/Save/Delete. |
| EQ AUTO click; plain A while EQ focused | Toggles/publishes/persists a flag and lights the cell; no autoload lookup. `rustamp-ui/src/equalizer.rs:552-555,582-585` (`b3d01f8f`); `rustamp-audio/src/engine.rs:1028-1033` (`d39ce3f1`). `git grep` over app/audio/DSP found no preset loading consumer. | Treat autoload as unsupported too, not as working merely because the lamp and stored flag change. |

The actual Windows menu is native, not the retired self-drawn popup:
`crates/rustamp-app/src/window.rs:854-878` and `menu.rs:10-16,112-134`.
Its submenu leaves all use `enabled: true`
(`crates/rustamp-ui/src/menu.rs:843-854`, `2554c3e8`), so the plugin leaves
really are active-looking/reachable, not merely dead entries in an unused table.
Notes are printed to stderr (`crates/rustamp-app/src/window.rs:1756-1757`),
not visible unavailable explanations in the packaged GUI.

The live analyzer/scope click (`crates/rustamp-ui/src/behaviour.rs:520-523`)
cycles working UI-local modes; it is deliberately left alone. EQ ON, bands and
preamp have real signal effects and are not bundled with unsupported presets.
Load/Save/Delete are constants for a popup that never opens; they are not
three additional currently reachable menu operations. Modified Ctrl+S is
ignored by `equalizer.rs:572-575`, not a newly discovered working preset loader.
Unrelated menu skeletons (Library, bookmarks, Video, etc.) belong to other
readiness work, not an excuse to broaden this lane's implementation scope.

## 4. Exact follow-up seams, scope, and non-work

After the release decision, apply one availability policy consistently in:

- `crates/rustamp-ui/src/menu.rs`: entries and `native_main_menu` must emit
  disabled native leaves or approved explicit unavailable labels; don't alter
  only the retired bitmap renderer.
- `crates/rustamp-ui/src/behaviour.rs` / `equalizer.rs` / `eq_render.rs`:
  V/PRESETS/AUTO hit behavior and their visible presentation must agree.
- `crates/rustamp-ui/src/shortcuts.rs` and app command handling:
  keyboard, menu, direct-command and host-forwarded routes must not bypass the
  approved unavailability response.
- `crates/rustamp-app/src/dialogs.rs` / `crates/rustamp-prefs/src/lib.rs`:
  keep the existing no-settings warning if the frame remains reachable;
  synchronize stale crate/app documentation only with the approved behavior.

No out-of-owned-path code change is required just to gate the player UI.
Do **not** repair the engine tap, DLL loader, projectM wrapper or frozen core
contracts as part of this card. Implementing visualization DLL lifecycle would
require the unowned `rustamp-plughost` preflight/loader/registry and S09/S08
decisions; that is explicitly not proposed as an honesty fix. Updating the
historical `BETA-readiness.md` or packaging limitations text is likewise outside
this lane's owned card/source paths and was not done.

No visualizer, plugin loading, presets, autoloading, S16 settings, dependency,
download, installation, commit or git write operation was implemented/performed.
No forbidden source/SDK/disassembly/research tree was read. Only clean exported
specs and Rustamp's own source/history were used; no third-party code copied.

## 5. Tests and real-desktop acceptance still required

**Tests added/changed: none**, because implementation stopped at the requested
decision boundary. No existing test was weakened. No golden was blessed or
changed. No `cargo test`, test executable, player, VM or VM broker was run.

Once policy is approved, tests must fail if an unsupported native leaf is
enabled/unlabelled, if a mouse/key/direct command reaches a placebo path, or
if AUTO/PRESETS/V render identically to their active appearance while declared
disabled. Existing state-change tests prove only dispatch/flags, not autoload
or DLL visualization. Keep those facts distinct rather than relabel them as
feature acceptance.

Only an interactive real desktop can confirm readable warnings, visibly
disabled menu and skinned controls in the Rust theme/lookalike/external skins,
keyboard focus/accelerator behavior, no phantom pressed/toggled feedback,
and that the packaged binary really exposes the approved policy. Render
geometry and successful builds cannot establish this. The orchestrator owns
the VM suite and desktop evidence; no new screenshot acceptance is claimed.

## 6. Gate and boundary receipts

Commands run with `Set-Location` first and `CARGO_TARGET_DIR` pinned to this
worktree's `target`; `CARGO_NET_OFFLINE=true` prevents downloads. Each Cargo
exit code is captured immediately into its own PowerShell variable.

- `cargo fmt --all -- --check`: **0** (`$fmtExit`).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **0** (`$clippyExit`, 44.40 s).
- `cargo build --workspace`: **0** (`$buildExit`, 27.47 s).

The first two shell attempts during the server restart returned transport-level
exits 4/66 without command output; neither was counted as a Cargo gate result.
The successful fmt receipt above is the explicit post-restart Cargo result.
The first background build was cancelled by another server restart; its
incomplete run is not a passing gate. The subsequent foreground build returned
the explicit 0 above. `git diff --check` returned 0. This card does not claim
BETA-10 done.

Worktree status: only `?? docs/tasks/W26-beta10-honest-skeletons.md` (this new
card); no tracked source changes. The final report gives its measured line count.

**Canonical-tree boundary check did not return empty.** The read-only
`git -C D:\code\Rustamp status --short` reported the following unrelated
working changes during this lane. None was edited, reverted or adopted here;
an empty canonical status cannot honestly be supplied by this lane:

```text
 M Cargo.lock
 M THIRD_PARTY.md
 M crates/rustamp-app/Cargo.toml
 M crates/rustamp-app/src/dialogs.rs
 M crates/rustamp-app/src/window.rs
 M crates/rustamp-platform/Cargo.toml
 M crates/rustamp-platform/src/lib.rs
 M crates/rustamp-ui/src/menu.rs
 M crates/rustamp-ui/tests/menu_render.rs
?? crates/rustamp-app/build.rs
?? crates/rustamp-platform/src/prefs_dialog.rs
```

These are not evidence about this worktree's reviewed HEAD; they must not be
silently incorporated into this audit. The orchestrator must reconcile its own
working tree and the missing release decision before claiming BETA-10 closed.
