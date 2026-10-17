# Task card M3hB — Resolve Group B CODEX Review Findings (3, 4, 5)

**Background**
An independent review (OpenAI CODEX, review-only) of the Rustamp out-of-process plugin host implementation identified evidence contradictions against spec `specs/S10-ipc.md` in `rustamp-plughost`. This task card documents the investigation, spec traceability, architectural decisions, and code documentation/tests for Group B (findings 3, 4, and 5).

**Allowed sources**
- `specs/S10-ipc.md` (the measured table — every value must trace to it)
- `specs/S09-plugin-abi.md` (audited ABI)
- `docs/rfc/0006-out-of-process-hosting.md`
- `docs/rfc/0009-ipc-version-constant.md`
- Crate `crates/rustamp-plughost/src/`

**Clean room rules**
Never read Winamp source, SDK headers, disassembly, the 2.95 reference binary, `rustamp-research`, or `rustamp-refbin`.

---

## 1. Per-Finding Summary Table

| Finding | Summary | Implemented Today (file:line) | Contradicted S10 Row(s) / Contract | Decision | Action Taken |
|---|---|---|---|---|---|
| **3** | Mutating IPCs (102/135) report effects they do not cause; setters change only local surface until next snapshot overwrites them. | `crates/rustamp-plughost/src/ipc.rs:750` (`101 \| 121 \| 102 \| 135 => CommandEffect { id }`); `ipc.rs:781-783, 790-793, 824-834, 842-843, 850-851`; `crates/rustamp-plughost/src/state_bridge.rs:97-133` (`apply_player_state` overwrites all setter fields). | S10 §3 Row 102 (`IPC_STARTPLAY`, +359 px main window effect), Row 135 (`IPC_RESTARTWINAMP`, process restart); S10 §6 (setters land and persist); S10 §2 & §11 (answerer model unratified; `WinampIpc` placeholder; RFC 0005 O13 open). | **Not enough audited evidence / spec contract to fix.** Out-of-process reverse command channel is an architectural protocol decision requiring an RFC. Inventing messages across IPC would violate clean-room and RFC rules. | **Behavior unchanged.** Added explicit code comments in `ipc.rs` and `state_bridge.rs` documenting the missing reverse channel. Added unit tests in `ipc.rs` and `state_bridge.rs` pinning the local command effect and snapshot overwrite behavior. Defined smallest next step: RFC for reverse commands. |
| **4** | Unmeasured replies asserted as established (sent volume/pan returns, -666 on shuffle/repeat, 126 raw Hz pinned to 44100, 242 always available). | `crates/rustamp-plughost/src/ipc.rs:781-784, 790-794` (returns new value); `ipc.rs:839-844, 847-852` (-666 query branch on 252/253); `ipc.rs:804`, `state_bridge.rs:30-31, 113` (sample rate in raw Hz); `ipc.rs:313-323, 720, 897` (242 `Fixed(1)`, `inet_available: true`). | S10 §0.2 E4, §6, §13.11 (sent volume/pan return unmeasured); S10 §3 Rows 250–253 (-666 measured only for 122/123; 250/251 are dedicated getters; 252/253 are setters); S10 §3 Row 126 & §13.2 (stream info while playing is unmeasured); S10 §3 Row 242 & §13.7 (242 is provisional/networking-dependent, not a fixture property). | **Not enough audited evidence to fix.** The alternative return values in 2.95 are unmeasured; changing them would be speculative guessing without empirical evidence. | **Behavior unchanged.** Clarified table notes and match arm comments in `ipc.rs` and `state_bridge.rs` explicitly marking all four replies as unmeasured fallbacks / unverified hypotheses. Added unit tests pinning fallback behaviors and documenting their unmeasured status. Defined smallest next steps for sandbox capture jobs. |
| **5** | Window/menu getters (260/281) return zeros while S10 measured nonzero handles. | `crates/rustamp-plughost/src/ipc.rs:854-863` (260 returns `RuntimeHandle { handle }`); `ipc.rs:874-877` (281 returns `RuntimeHandle { handle }`); `ipc.rs:714-719` (`eq_hwnd`, `playlist_hwnd`, etc. default to 0); `host_window.rs:353` (only `main_hwnd` set). | S10 §3 Row 260 (`IPC_GETWND`, distinct per-boot handles 66262, 66264, etc.); S10 §3 Row 281 (`IPC_GET_HMENU`, distinct per-boot menu handles 262789, 328057); S10 §8.1 & §13.8 ("how our host supplies handles valid in the plugin's process is an M3 architecture decision, not a 2.95 fact"). | **Not enough spec decision to fix.** 2.95 ran in a single 32-bit process with native handles. Rustamp runs a 64-bit GUI player and 32-bit out-of-process plughost. Inventing fake nonzero numbers risks crashing plugins that pass them to Win32 APIs. | **Behavior unchanged.** Documented in `ipc.rs`, `host_window.rs`, `state_bridge.rs`, and `test_plugin_ours.rs` that returning 0 contradicts S10 nonzero measurements and is a placeholder pending an M3 design decision (S10 §13.8). Added unit test pinning 0 default. Defined smallest next step: M3 architectural RFC. |

---

## 2. Detailed Findings, Decisions, and Traceability

### Finding 3 — Mutating IPCs and Setter Snapshot Overwrites

#### 1. What is implemented today
- In `crates/rustamp-plughost/src/ipc.rs:750`:
  `101 | 121 | 102 | 135 => IpcAnswer::CommandEffect { id }`
  When evaluated through `raw()` (lines 624–626), this yields `UNKNOWN_REPLY = 1`.
  `IPC_STARTPLAY` (102) and `IPC_RESTARTWINAMP` (135) emit no command and trigger no action in the player engine or host process.
- In `crates/rustamp-plughost/src/ipc.rs:777-853`:
  Setters modify only the in-memory fields of `IpcSurface`:
  - 122 (`IPC_SETVOLUME`): `self.volume = v;` (line 782)
  - 123 (`IPC_SETPANNING`): `self.pan = signed;` (line 792)
  - 128 (`IPC_SETEQDATA`): `self.eq_bands[sel] = value;` (lines 824–834)
  - 252 (`IPC_SET_SHUFFLE`): `self.shuffle = wparam != 0;` (line 842)
  - 253 (`IPC_SET_REPEAT`): `self.repeat = wparam != 0;` (line 850)
- In `crates/rustamp-plughost/src/state_bridge.rs:97-133`:
  When the next `Message::PlayerStateUpdate` snapshot arrives from `rustamp.exe` over the control pipe, `apply_player_state` unconditionally overwrites all these fields from `PlayerState`:
  ```rust
  surface.volume = state.volume;
  surface.pan = i32::from(state.balance);
  ...
  surface.shuffle = state.shuffle;
  surface.repeat = state.repeat;
  ```
  Any value set by a plugin through the host window is overwritten on the very next state update.
- In `crates/rustamp-plughost/src/control.rs:59-64`:
  The control channel reader intentionally ignores `WinampIpc` because no reverse command or reply protocol exists in `rustamp-ipc`.

#### 2. Contradicted S10 rows and contracts
- **S10 §3 Row 102 (`IPC_STARTPLAY`):** Measured in run B3 step `play_action` with a +359 px main window redraw (playback started, transport and position updated). `rustamp-plughost` answers `CommandEffect` but causes no playback.
- **S10 §3 Row 135 (`IPC_RESTARTWINAMP`):** Measured in run B3 step `restart_action` (exit 3 `GetClientRect failed` mid-restart) proving that 2.95 restarts the process. `rustamp-plughost` answers `CommandEffect` but does not restart the player.
- **S10 §3 Rows 122, 123, 128, 252, 253:** S10 §6 proves that setter writes landed and persisted across subsequent queries (e.g., volume 64/0/255 read-backs, pan -1 read-back, EQ 0 read-back with +431 px redraw, shuffle/repeat toggling 1 and 0). In `rustamp-plughost`, setters do not persist across engine state updates.
- **S10 §2 ("The answerer model") & §11:** S10 explicitly classifies engine-state IPCs as `answered-by-player`, but notes that this table is a "proposal for ratification, not a measured fact", and that `Message::WinampIpc` in `rustamp-ipc` is an unratified placeholder whose relationship to `CommandToHost` is open RFC 0005 O13.

#### 3. Decision
**Not enough audited evidence / spec contract to fix.**
While S10 proves that 102, 135, and setters produce persistent engine effects in Winamp 2.95, our architecture separates the 32-bit plugin host from the 64-bit engine process. Forwarding plugin-invoked mutations to the player requires a bidirectional command channel in `rustamp-ipc`. No such message or channel currently exists, S10 §2 is unratified, and RFC 0005 O13 is open.
Per clean-room and architectural rules, an implementer must not invent protocol messages without an RFC. Changing the local behavior without a player-side contract would be an unauthorized guess.

#### 4. Action and smallest next step
- **Action:** Retained existing local surface mutation and `CommandEffect` return. Added explicit documentation comments in `ipc.rs` and `state_bridge.rs`. Added tests in `ipc.rs` (`finding_3_mutating_ipcs_return_command_effect_without_engine_channel`) and `state_bridge.rs` (`snapshot_overwrites_local_setter_mutations_pending_reverse_channel`).
- **Smallest next step:** An RFC defining reverse commands across `rustamp-ipc` (e.g., `Message::CommandFromHost(rustamp_proto::command::Command)` or ratifying S10 §2 `WinampIpc`), enabling `plughost` to notify `rustamp-app` when mutating IPCs (102, 122, 123, 128, 135, 252, 253) are invoked.

---

### Finding 4 — Unmeasured Replies Asserted as Established

#### 1. What is implemented today
1. **Sent volume and pan returns:**
   In `crates/rustamp-plughost/src/ipc.rs:781-784, 790-794`:
   When `wparam != -666`, `answer(122, wparam)` returns `IpcAnswer::Value(v as u32)` (the new volume), and `answer(123, wparam)` returns `IpcAnswer::Value(signed as u32)` (the new pan).
2. **-666 on shuffle/repeat:**
   In `crates/rustamp-plughost/src/ipc.rs:839-844, 847-852`:
   Arms 252 and 253 check `if wparam == -666` and return `u32::from(self.shuffle)` / `u32::from(self.repeat)` without modifying the toggle state.
3. **126 raw Hz pinned to 44100:**
   In `crates/rustamp-plughost/src/ipc.rs:804`: Mode 0 returns `IpcAnswer::Value(self.sample_rate)`.
   In `crates/rustamp-plughost/src/state_bridge.rs:30-31, 113`:
   `surface.sample_rate = state.stream.map(|s| s.sample_rate_hz).unwrap_or(0);`
   Tests in `state_bridge.rs:247` and `plughost_e2e.rs:186` assert `44_100`.
4. **242 always available:**
   In `crates/rustamp-plughost/src/ipc.rs:313-323`:
   Row 242 declares `measured: MeasuredReply::Fixed(1)`, `measured_value: Some(1)`.
   Line 720 defaults `inet_available: true`, and line 897 returns `Value(u32::from(self.inet_available))`.

#### 2. Contradicted S10 rows and contracts
1. **Sent volume/pan returns:**
   Contradicts S10 §0.2 E4 ("the setter steps were posted by design, so the set calls' own replies were never read"), S10 §3 rows 122 and 123 ("the sent set reply itself is unmeasured (§13.11)"), S10 §6 ("Not settled (E4): the set calls' own replies. The setter steps were posted by design, so a sent 122/123 was never read: whether it returns the old or the new value is open (§13.11)"), and S10 §13.11.
2. **-666 on shuffle/repeat:**
   Contradicts S10 §3 rows 250, 251, 252, and 253. In S10, 250 (`IPC_GET_SHUFFLE`) and 251 (`IPC_GET_REPEAT`) are the measured getters; 252 and 253 are setters. `-666` is measured strictly for volume (122) and pan (123) (S10 §1.5, §3, §5.6). Winamp 2.95 has no measured `-666` query on 252/253 (a negative non-zero wParam in 2.95 is treated as a truthy setter argument).
3. **126 raw Hz pinned to 44100:**
   Contradicts S10 §3 row 126 ("provisional — stopped, no stream; a playing reference is needed (§13.2)") and S10 §13.2 ("The state-dependent ids in a playing state. 105..., 126 (sample rate/bitrate/channels while a stream is open)... Settling capture: a loaded and playing reference pass"). The return value of mode 0 while playing (raw Hz e.g. 44100 vs kHz e.g. 44) was never measured.
4. **242 always available:**
   Contradicts S10 §3 row 242 ("provisional — sandbox-networking-dependent; not a fixture property, do not export it as one (§13.7)") and S10 §13.7. `Fixed(1)` in `IPC_TABLE` claims an established fixture invariant where S10 explicitly forbids exporting it as one.

#### 3. Decision
**Not enough audited evidence to fix.**
None of these four return behaviors can be replaced with an audited 2.95 measurement because the settling reference runs have not yet been performed:
- Whether sent volume/pan returns the old value or new value in 2.95 is open (S10 §13.11).
- What 2.95 returns when `-666` is sent to 252/253 is unmeasured.
- Whether 126 mode 0 returns raw Hz or kHz while playing is open (S10 §13.2).
- Whether 242 returns 0 or 1 when disconnected from network is open (S10 §13.7).
Inventing alternative numbers without measurements would violate clean-room integrity.

#### 4. Action and smallest next step
- **Action:** Left runtime return behavior unchanged to avoid breaking callers or guessing. Updated code comments in `ipc.rs` (table rows and `answer()` match arms) and `state_bridge.rs` explicitly identifying each of these four items as unmeasured fallbacks / provisional values. Added unit tests in `ipc.rs` pinning the current fallback behaviors and documenting their unmeasured status.
- **Smallest next steps:**
  1. Sent volume/pan: A sandbox capture step sending 122 and 123 synchronously with `expect_reply` against a known state (S10 §13.11).
  2. -666 on shuffle/repeat: A capture probe sending `wParam = -666` to 252/253 to determine if 2.95 sets the toggle or treats it as an error.
  3. 126 mode 0: A capture job in the test VM querying id 126 while playing a track from `rustamp-testmedia` (S10 §13.2).
  4. 242: A capture probe in a sandbox with network adapter disabled reading id 242 (S10 §13.7).

---

### Finding 5 — Window/Menu Getters (260/281) Return Zeros

#### 1. What is implemented today
- In `crates/rustamp-plughost/src/ipc.rs:854-863`:
  ```rust
  260 => {
      let handle = match wparam {
          0 => self.eq_hwnd,
          1 => self.playlist_hwnd,
          2 => self.minibrowser_hwnd,
          3 => self.video_hwnd,
          _ => 0,
      };
      IpcAnswer::RuntimeHandle { id, handle }
  }
  ```
- In `crates/rustamp-plughost/src/ipc.rs:874-877`:
  ```rust
  281 => IpcAnswer::RuntimeHandle {
      id,
      handle: self.menu_handle,
  }
  ```
- In `crates/rustamp-plughost/src/ipc.rs:714-719`:
  `eq_hwnd`, `playlist_hwnd`, `minibrowser_hwnd`, `video_hwnd`, and `menu_handle` default to `0`.
- In `crates/rustamp-plughost/src/host_window.rs:353`:
  Window creation sets `self.main_hwnd = hwnd.0 as usize as u32;`, leaving all other window and menu handles at `0`.
- In `crates/rustamp-plughost/src/state_bridge.rs:58-62, 130-132`:
  Runtime handles are excluded from state bridge updates because no protocol message carries them.
- Calling 260 or 281 returns `RuntimeHandle { handle: 0 }`, evaluating to `raw() == 0`.

#### 2. Contradicted S10 rows and contracts
- **S10 §3 Row 260 (`IPC_GETWND`):**
  Measured in runs R2 and R3 as real, distinct, per-boot HWNDs (`EQ 66262, playlist 66264, video 66266, minibrowser 66268/66270`). `plughost` returns 0.
- **S10 §3 Row 281 (`IPC_GET_HMENU`):**
  Measured in runs R2 and R3 as a non-null per-boot HMENU (`262789` in R2, `328057` in R3). `plughost` returns 0.
- **S10 §8.1 & §13.8:**
  S10 explicitly notes that these measurements are **shape-only**, and that:
  > *"how our host supplies handles valid in the plugin's process is an M3 architecture decision, not a 2.95 fact. Settling: the M3 design decision, then a plugin exercising 260/281 against our host."*

#### 3. Decision
**Not enough spec decision to fix.**
In Winamp 2.95, Winamp ran in a single process that owned real Win32 HWNDs for each window and a native HMENU.
In Rustamp, the GUI windows belong to the 64-bit `rustamp.exe` process, while plugins run in the 32-bit `rustamp-plughost32.exe`. An HMENU cannot be shared across processes without IPC orchestration or local proxy menus. Providing fake nonzero numbers without a backing Win32 object is dangerous: if a plugin passes a fake handle to Win32 functions (`GetWindowRect`, `TrackPopupMenu`, `SendMessage`), the process may crash or fault (similar to Finding 2 for id 800).
Until an M3 architectural decision is ratified, returning 0 remains the only safe implementation.

#### 4. Action and smallest next step
- **Action:** Retained 0 defaults. Updated comments in `ipc.rs` (table rows and `answer()`), `host_window.rs`, `state_bridge.rs`, and `test_plugin_ours.rs` documenting that returning 0 contradicts S10 nonzero measurements and is a placeholder pending an M3 architectural decision per S10 §13.8. Added unit test `finding_5_window_and_menu_handles_default_to_zero_pending_m3_decision`.
- **Smallest next step:** An M3 design RFC deciding whether `plughost` creates local hidden 32-bit proxy HWNDs and an emulated HMENU, or bridges HWNDs from `rustamp-app`.

---

## 3. Code Changes Made

1. `crates/rustamp-plughost/src/ipc.rs`:
   - Updated `IPC_TABLE` row notes for 102, 122, 123, 126, 135, 242, 252, 253, 260, 281 to document S10 evidence status and pending decisions.
   - Added explanatory comments in `IpcSurface::answer()` arms for 101/121/102/135, 122, 123, 126, 128, 252, 253, 260, 281, and 242.
   - Added unit tests in `mod tests`:
     - `finding_3_mutating_ipcs_return_command_effect_without_engine_channel`
     - `finding_4_sent_volume_and_pan_fallback_returns_new_value`
     - `finding_4_shuffle_and_repeat_unmeasured_minus_666_fallback`
     - `finding_4_stream_info_playing_sample_rate_fallback`
     - `finding_4_inet_available_provisional_reply`
     - `finding_5_window_and_menu_handles_default_to_zero_pending_m3_decision`
2. `crates/rustamp-plughost/src/state_bridge.rs`:
   - Updated module documentation with section `# Mutating IPCs and Setter overwrites (Finding 3)` and notes on Findings 4 and 5.
   - Added unit test: `snapshot_overwrites_local_setter_mutations_pending_reverse_channel`.
3. `crates/rustamp-plughost/tests/test_plugin_ours.rs`:
   - Updated comments on volume setter return value and handle selector defaults to document their S10 status.

---

## 4. Verification

- `cargo fmt --all -- --check`: Clean (no formatting differences).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Clean (0 warnings).
- `cargo build --workspace`: Clean (built 0 errors).
- Zero external worktree modifications confirmed via `git status --short`.

## 5. Hard Rule Confirmations

- **Ran no test:** No `cargo test` command was executed on the host machine.
- **Opened no window:** No window, GUI, or child console window was created.
- **Made no commit:** No git write/commit command was executed.
- **Wrote only inside worktree:** All edits were strictly contained within `crates/rustamp-plughost/` and `docs/tasks/M3hB-groupb.md`.
