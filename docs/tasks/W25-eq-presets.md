# W25 - Equalizer presets: spec boundary and implementation stop

**Status:** blocked on specification; no preset implementation submitted.
**Inspected revision:** `c88396c4336fecd2bc72e3ef62a19474f0ecffa3`.
**Lane:** clean-room, 2026-10-19.
**Owned paths:** `crates/rustamp-ui/**`, `crates/rustamp-app/**`, this card.
**Write boundary:** only the lane worktree
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave125-eqpresets`;
write only your own files, and nothing under `D:\code\Rustamp`.

## Approval supplied by the specs

- S07 section 4.1 settles the Presets button footprint at (217,18,44,12)
  and the 528-pixel held/released change, but not a distinct held sheet rect.
- S07 section 4.2 (`specs/S07-eq-dsp.md:251-268`) settles a same-process,
  native `#32768` popup, client 176 x 105 and outer 182 x 111, with exactly
  `Load`, `Save`, `Delete`, in that order, each with a submenu arrow.
- S04 section 6 (`specs/S04-menus.md:333-356`) corroborates those rows and
  arrows: no checks, separators or greyed rows in the captured fresh state.
- The measured popup sizes are not universal layout constants: S04 section 8
  (`specs/S04-menus.md:390-425`) identifies system-drawn menus captured at
  DPI 144 (150%); system font and DPI determine the actual metrics.
- S07 section 1 mentions S opening the menu and Ctrl+S loading a preset;
  a shortcut identity does not specify a load operation or dialog contents.

## Verify readiness against the code, not its old line references

`docs/tasks/BETA-readiness.md:92` still correctly says presets only record an
unimplemented note. Its `app.rs:365-379` reference is stale, not its finding.
`git blame` attributes that readiness sentence to `0a06d661`.

The current path is:

1. `crates/rustamp-ui/src/equalizer.rs:557-559` queues `EqMenuRequest::Presets`
   on button release; `:587-589` does the same for S.
2. `crates/rustamp-app/src/app.rs:1260-1267` consumes the request through
   `request_eq_presets_menu`.
3. `crates/rustamp-app/src/app.rs:751-766` only records the note and returns
   the three labels. It stores no popup request for the presenter.
4. The existing test at `crates/rustamp-app/src/app.rs:2683-2700` explicitly
   pins that stub. The UI entries/constants at
   `crates/rustamp-ui/src/equalizer.rs:380-413` are not an OS popup.

`git blame -L 751,766 -- crates/rustamp-app/src/app.rs` attributes the
unchanged request stub to `b3d01f8f` (Add the equalizer window).
`2554c3e` added the live native **main** menu, not an EQ preset popup:
`crates/rustamp-app/src/menu.rs:101-114` and
`crates/rustamp-app/src/window.rs:832-887` supply and drain that main menu.
The EQ key/mouse presenter paths still call the app stub. Searching the owned
crates and exported specs found no other implemented EQ preset load/save path.
Thus the task's premise that an EQ popup already exists is not true at this
revision; the headless request and the main-menu popup must not be confused.

## Open questions - stop, do not choose defaults

S07 section 4.2 and section 12 item 12 (`:586-589`) explicitly leave the
following open; S04 section 10 item 10 (`:488-491`) does not close them:

1. What are the complete Load / Save / Delete submenu labels, ordering,
   command identities, available states and effects?
2. Which presets, if any, ship, with what exact band and preamp values?
3. May a preset be edited, renamed, overwritten or deleted? What are the
   confirmation, cancellation, duplicate-name and invalid-input rules?
4. Where are user presets stored? What are the file format and encoding?
   `.eqf` and `winamp.q1` are planning topics, not specified byte formats.
5. Does loading change `preamp`, `eq_data`, `autoload_eq`, `use_eq`, or an
   on-disk preset? Does closing the EQ window save a preset?
6. What selects an autoload preset for a track, and what happens on no match?

**Implementation stop:** no built-in preset catalogue, editable preset model,
storage path, file parser/writer, import/export flow, AUTO behaviour or
load/save/delete operation is authorized by these specs. A top-level popup is
specified, but choosing empty child menus, placeholder rows, disabled parents
or invented actionable submenus would decide its currently open contents or
behaviour. No such substitute was introduced. The owner requested stopping
at this boundary; functional presets remain blocked, not completed.

## Follow-up once the specification closes the questions

- Add the approved EQ popup request/drain path in the owned app/UI crates,
  using the existing native popup facility and the EQ window as owner.
  Native system menus provide `#32768`, readable system text and submenu
  arrows without creating a captioned application window. Do not revive the
  retired self-drawn main-menu surface or hard-code 150% menu metrics at 100%.
- If an application window is later needed for an approved dialog, use the
  shared `rustamp_ui::skinned_window_attributes`, never default attributes.
- No required out-of-scope code change was established. If an approved
  solution needs a core/proto contract or platform change, record its exact
  requirement and obtain the required RFC/ownership before editing it.

## Verification and remaining desktop acceptance

- No production code changed; only this card was added. No tests were added,
  weakened, removed or run. Existing stub assertions remain unchanged rather
  than being silently redefined as preset acceptance.
- No rendered expectation or golden image was created, updated or blessed.
- The orchestrator owns VM execution; this lane starts no GUI, VM or broker.
- Required host gates (each `$LASTEXITCODE` captured immediately into its own
  variable): `cargo fmt --all -- --check`: **0**;
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **0**;
  `cargo build --workspace`: **0**. All ran offline with `CARGO_TARGET_DIR`
  explicitly inside this worktree. These compile/check the unchanged code,
  not functional presets; clippy checked test targets but executed no tests.
- A real interactive VM desktop must eventually confirm menu appearance,
  readability, native arrows, absence of a native caption, placement relative
  to the EQ at 1x/2x and relevant DPI, actual opening from mouse/S, dismissal,
  focus return, and the approved submenu actions. No EQ popup exists here to
  claim a visual inspection or a visual defect from source alone.
- Clean-room: only repository docs/specs and Rustamp implementation were read;
  no Winamp source, SDK, disassembly, research tree or reference binary was
  accessed. No dependencies, downloads, git writes or commits.
