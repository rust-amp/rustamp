# Task card N13 — RFC 0010 contract application and release version 0.1.0-beta.1

**Scope**: `crates/rustamp-ui/**`, `crates/rustamp-app/**`, `Cargo.toml`, `docs/rfc/0010-load-playlist-command.md`, `docs/tasks/N13-rfc0010-version.md`.

## 1. Context and Owner Decisions

Two owner decisions are closed in this lane:
1. **RFC 0010 is Accepted**: The Load Playlist dialog command and contract are formally settled. The core command `Command::OpenPlaylist` triggers opening the dialog; dialog confirmation carries a file path (`Option<PathBuf>`) rather than an engine mutation command; the app loads the playlist with replace semantics and surfaces skip reports.
2. **First Release Version Number**: Following `docs/tasks/RELEASE-plan.md` §3 (the accepted versioning rule), the product receives its first SemVer number: `0.1.0-beta.1`.

## 2. Changes Implemented

### 2.1 RFC 0010 Contract Application and Dead Path Removal

Prior to this task, the app layer sat beside the `DialogManager` contract rather than following it:
- In `crates/rustamp-ui/src/dialogs.rs`, two confirmation methods existed:
  - An older path-less method: `pub fn confirm_load_playlist(&self) -> Option<Command> { None }`.
  - The path-based method: `pub fn confirm_load_playlist_path(&self) -> Option<PathBuf>`.
- The path-less `confirm_load_playlist` was dead: under RFC 0010, dialog confirmation does not emit a core `Command` (mapping to `AddItems` was explicitly rejected to avoid imposing append semantics).
- In `crates/rustamp-app`, the N7 implementation had created duplicate fields on `App` (`playlist_dialog_text: String`, `playlist_dialog_path: Option<PathBuf>`), duplicating path staging, file dropping, and text tracking instead of delegating to `self.dialogs: DialogManager`.

**Changes:**
1. **Removed Dead Path in `rustamp-ui`**: Removed `DialogManager::confirm_load_playlist(&self) -> Option<Command>`. Updated associated unit tests to verify `confirm_load_playlist_path` yields `None` when no input/dialog is active.
2. **Eliminated Duplicate State in `rustamp-app`**: Removed `playlist_dialog_text` and `playlist_dialog_path` fields from `App`.
3. **Wired App to `DialogManager`**:
   - `App::stage_playlist_path` routes to `self.dialogs.stage_playlist_path(path)`.
   - `App::dialog_drop_file` routes to `self.dialogs.drop_playlist_path(path)` when `DialogKind::LoadPlaylist` is open.
   - `App::playlist_text` and `set_playlist_text` read/write `self.dialogs`.
   - `App::dialog_accepts_text` delegates to `self.dialogs.accepts_text()`.
   - `App::dialog_text` delegates to `self.dialogs.type_text(text)`.
   - `App::load_playlist_lines` reads staged pick and text from `self.dialogs`.
   - `App::confirm_load_playlist` queries `self.dialogs.confirm_load_playlist_path()`; if a path is present, loads the file via `App::load_playlist_file(&path, true)` and closes the dialog; if absent (empty or whitespace), returns `false` leaving the dialog open.
   - `App::dialog_key`: `Key::Backspace` is handled directly by `self.dialogs.key`; `Key::Digit`, `Key::Letter`, and `Key::Space` route to `self.dialogs.type_text`.
4. **Preserved N7 Behavior & Skip Report**:
   - Queue replacement semantics (replacing existing tracks with surviving entries).
   - Track 0 playback initiation upon confirmation.
   - Structured skip reporting recorded on `App::last_playlist_report`.
   - Diagnostic notes emitted to `notes` (surfaced on stderr in windowed mode).
   - Error handling leaving queue untouched and setting `last_error` and `Event::Error`.
5. **RFC 0010 Status**: Updated `docs/rfc/0010-load-playlist-command.md` with `Status: **Accepted** (2026-10-18).`.

### 2.2 Version Stamping

- Set `[workspace.package].version = "0.1.0-beta.1"` in `Cargo.toml`.
- Every member crate in the workspace specifies `version.workspace = true`, inheriting this version.
- The user-facing About dialog (`crates/rustamp-app/src/dialogs.rs:336`) automatically surfaces `CARGO_PKG_VERSION` as `"Rustamp 0.1.0-beta.1"`.
- What a later bump would touch:
  - `Cargo.toml` (`[workspace.package].version`).
  - When packaging lane R1 lands, the packager will stamp this into package manifests (`manifest.json`) and enforce folder naming agreement (`rustamp-<version>+<date>.<short-commit>`).
  - Git release tags and `CHANGELOG` / release notes.
  - The three compatibility integers (`PROTOCOL_VERSION = 4`, `CONFIG_VERSION = 1`, and manifest `schema_version = 2`) follow their own independent bump disciplines; any wire protocol or configuration schema change forces at least a MINOR product version bump pre-1.0.

## 3. Tests

- **Unit tests in `rustamp-ui`**:
  - `confirming_the_load_playlist_dialog_yields_no_path_without_input`: verifies `confirm_load_playlist_path` returns `None` without input or when other dialogs are open.
  - `a_playlist_path_can_be_typed_staged_or_dropped`: verifies path typing, staging, and dropping in `DialogManager`.
  - `the_load_summary_names_counts_and_every_skip`: verifies skip report formatting helper.
- **Unit tests in `rustamp-app`**:
  - `about_dialog_body_reflects_workspace_version`: verifies the About dialog displays `Rustamp 0.1.0-beta.1`.
  - `load_playlist_uses_dialog_manager_contract`: verifies staging and dropping files update `DialogManager::confirm_load_playlist_path`, and dialog closing resets state.
  - Existing N7 tests verified: `good_playlist_file_loads_and_plays`, `playlist_file_with_bad_entry_loads_surviving_and_reports_skips`, `unsupported_playlist_file_reports_error_and_changes_nothing`, `load_playlist_dialog_text_input_and_display`, and `the_load_playlist_dialog_keeps_its_confirmation_open`.
- **What only the VM can prove**:
  - Full execution of the test suite (the orchestrator executes `powershell -File docs\notes\vm-run-tests.ps1` in the Hyper-V guest).
  - Skinned window rendering, GUI interactions, and window drag/hit-test behavior (Group A window tests).
  - Real DLL plugin loading (Group D tests).
  - Out-of-process process supervision and real audio output (WASAPI).
  - Clean OS execution without developer toolchains.
