# Task card S10-probes — Probes for Unmeasured S10 IPC Replies

This document defines the exact measurement probes and execution plans for the five unproven IPC replies in the Rustamp project (clean-room reimplementation of Winamp 2.95). These items were identified in the CODEX review ([docs/tasks/M3h-codex-findings.md](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/docs/tasks/M3h-codex-findings.md), Group B, Findings 4 and 5), left as unmeasured fallbacks by the Group B lane ([docs/tasks/M3hB-groupb.md](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/docs/tasks/M3hB-groupb.md)), and catalogued as open questions in [specs/S10-ipc.md](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/specs/S10-ipc.md) (§13).

> [!IMPORTANT]
> **Clean-room rule:** This document is a design and measurement plan for the dirty room. It records no invented measurement results. Every value remains unknown until a real dirty-room run is executed and audited.

---

## 1. Sent Volume (`IPC_SETVOLUME`, 122) and Sent Pan (`IPC_SETPANNING`, 123) Returns

### 1.1 The exact question and code impact
- **The Question:** When an external caller or plugin *sends* `IPC_SETVOLUME` (id 122) or `IPC_SETPANNING` (id 123) synchronously via `SendMessageW` with a setter argument (`wParam != -666`), what does `SendMessageW` return?
  - Does it return the **new value** just set?
  - Does it return the **old / previous value** prior to the change?
  - Does it return a fixed status code (e.g., `1` for success, or `0`)?
  - Does `IPC_SETPANNING` return an 8-bit unsigned value (`0..255`), a signed 8-bit value sign-extended to 32 bits (`-127..127`), or something else?
- **Status in S10:** S10 §0.2 (E4), §3 (rows 122, 123), §6, and §13.11 explicitly record that in previous reference runs (R2/R3), setter calls were posted (`PostMessageW`) to test side effects, and query calls used `wParam = -666`. The return value of a *sent* setter was never read. [IDX] states "int (old or new value)".
- **What an answer changes in our code:**
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L781-L806):
    - Lines 781–793 (`122 =>`): Currently returns `IpcAnswer::Value(v as u32)` (the new volume). If 2.95 returns the *previous* volume, this must be changed to capture `let old = self.volume;` before assignment and return `IpcAnswer::Value(old as u32)`.
    - Lines 794–806 (`123 =>`): Currently returns `IpcAnswer::Value(signed as u32)` (the new signed pan). If 2.95 returns the previous pan, this must be changed to return `old as u32`. If the return representation is unsigned (`0..255`), line 804 must return `(b & 0xff) as u32`.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L1126-L1138):
    - Unit test `finding_4_sent_volume_and_pan_fallback_returns_new_value` currently asserts the unverified "returns new value" hypothesis. It must be updated to pin the measured 2.95 contract.

### 1.2 The probe
- **Target:** Main window (`class: "Winamp v1.x"`).
- **Window Message:** `WM_USER` (`0x0400` / `1024`). `lParam = 122` (Volume) or `123` (Pan).
- **Execution Mode:** Synchronous `SendMessageTimeoutW` with `expect_reply: true`.
- **Initial State:** Fresh launch (volume defaults to 0, pan defaults to 0, stopped, repeat/shuffle off).
- **Probe Steps (Volume):**
  1. `vol_pre`: Send `msg: 1024`, `wparam: -666`, `lparam: 122`, `expect_reply: "0"` (confirms baseline volume is 0).
  2. `vol_set_100`: Send `msg: 1024`, `wparam: 100`, `lparam: 122`, `expect_reply: "100"` (hypothesis: new value).
  3. `vol_set_150`: Send `msg: 1024`, `wparam: 150`, `lparam: 122`, `expect_reply: "150"` (hypothesis: new value).
  4. `vol_post`: Send `msg: 1024`, `wparam: -666`, `lparam: 122`, `expect_reply: "150"` (confirms current value is 150).
- **Probe Steps (Pan):**
  1. `pan_pre`: Send `msg: 1024`, `wparam: -666`, `lparam: 123`, `expect_reply: "0"` (confirms baseline pan is 0).
  2. `pan_set_64`: Send `msg: 1024`, `wparam: 64`, `lparam: 123`, `expect_reply: "64"` (hypothesis: new value).
  3. `pan_set_neg64`: Send `msg: 1024`, `wparam: -64`, `lparam: 123`, `expect_reply: "4294967232"` (hypothesis: new signed value `-64` zero-extended as `0xFFFFFFC0`).
  4. `pan_post`: Send `msg: 1024`, `wparam: -666`, `lparam: 123`, `expect_reply: "4294967232"`.

### 1.3 Distinguishing candidate answers
- **Volume:**
  - *Candidate A (New value):* `vol_set_100` returns `100`; `vol_set_150` returns `150`.
  - *Candidate B (Old value):* `vol_set_100` returns `0` (the initial baseline); `vol_set_150` returns `100` (the value prior to setting 150).
  - *Candidate C (Status code):* `vol_set_100` and `vol_set_150` both return `1` (or `0`).
- **Pan:**
  - *Candidate A (New value):* `pan_set_64` returns `64`; `pan_set_neg64` returns `4294967232` (signed `-64`) or `192` (unsigned byte).
  - *Candidate B (Old value):* `pan_set_64` returns `0` (initial baseline); `pan_set_neg64` returns `64`.
  - *Candidate C (Status code):* Returns `1` or `0`.
- **Misleading answer pitfall:** If the probe sets the *same* value that was already active (e.g., setting `0` when volume is already `0`), old value and new value are identical (`0 == 0`), making it impossible to distinguish Candidate A from Candidate B. The sequential transitions (`0 -> 100 -> 150`) guarantee non-zero deltas.

### 1.4 Pitfalls
- **Harness unsigned 32-bit formatting:** The capture harness reads the 32-bit reply as an unsigned decimal integer. A signed reply of `-64` is recorded as `4294967232`. An assertion of `"-64"` will fail with exit code 4.
- **Posting instead of sending:** Calling `PostMessageW` (omitting `expect_reply`) queues the message and returns `reply = ""` without reading the window procedure's return value.

### 1.5 Acceptance criterion and exact artefact
- **Artefact:** Rows in `run-manifest.json` for steps `vol_set_100`, `vol_set_150`, `pan_set_64`, and `pan_set_neg64`.
- **Acceptance Criterion:** The `reply` fields in `run-manifest.json` resolve deterministically to either Candidate A, B, or C, and agree across two consecutive runs.
- **Harness Readiness:** **Answerable with today's harness.** Standard `send-message` with `expect_reply: true` on `Winamp v1.x`.

---

## 2. The -666 Reply on Shuffle (`IPC_SET_SHUFFLE`, 252) and Repeat (`IPC_SET_REPEAT`, 253)

### 2.1 The exact question and code impact
- **The Question:** In Winamp 2.95, when `wParam = -666` is sent to `IPC_SET_SHUFFLE` (id 252) or `IPC_SET_REPEAT` (id 253), does it act as a query returning the toggle state without modifying it, or is `-666` evaluated as a non-zero boolean setter argument (`wParam != 0`) that forcibly enables shuffle/repeat?
- **Status in S10:** S10 §3 rows 250–253 define 250 (`IPC_GET_SHUFFLE`) and 251 (`IPC_GET_REPEAT`) as the dedicated getters, and 252/253 as setters. `-666` was measured strictly for volume (122) and pan (123) (S10 §1.5, §5.6). Winamp 2.95 has no measured `-666` query on 252/253.
- **What an answer changes in our code:**
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L853-L876):
    - Lines 856–857 (`if wparam == -666 { IpcAnswer::Value(u32::from(self.shuffle)) }`): If 2.95 does not support `-666` query mode on 252, this speculative branch must be removed.
    - Lines 868–869 (same for repeat on 253): Must be removed if unsupported.
    - Furthermore, if `-666` is evaluated as a setter argument in 2.95, whether it treats negative values as truthy (`wparam != 0`) or falsey (`wparam > 0`) must be aligned.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L1141-L1157):
    - Unit test `finding_4_shuffle_and_repeat_unmeasured_minus_666_fallback` must be updated or replaced.

### 2.2 The probe
- **Target:** Main window (`class: "Winamp v1.x"`).
- **Window Message:** `WM_USER` (`1024`). `lParam = 252` (Shuffle) and `253` (Repeat).
- **Execution Mode:** Synchronous `SendMessageTimeoutW` with `expect_reply: true`.
- **Initial State:** Fresh launch (shuffle = 0, repeat = 0).
- **Probe Steps (Shuffle):**
  1. `shuff_verify_off`: Send `msg: 1024`, `wparam: 0`, `lparam: 250`, `expect_reply: "0"` (confirms shuffle is OFF via dedicated getter).
  2. `shuff_probe_minus666`: Send `msg: 1024`, `wparam: -666`, `lparam: 252`, `expect_reply: "0"` (hypothesis: query mode returns current state 0).
  3. `shuff_readback_after_minus666`: Send `msg: 1024`, `wparam: 0`, `lparam: 250`, `expect_reply: "0"` (verifies if step 2 accidentally turned shuffle ON).
  4. `shuff_turn_on`: Send `msg: 1024`, `wparam: 1`, `lparam: 252`, `expect_reply: "1"` (sets shuffle to ON).
  5. `shuff_verify_on`: Send `msg: 1024`, `wparam: 0`, `lparam: 250`, `expect_reply: "1"`.
  6. `shuff_probe_minus666_when_on`: Send `msg: 1024`, `wparam: -666`, `lparam: 252`, `expect_reply: "1"`.
  7. `shuff_readback_after_minus666_on`: Send `msg: 1024`, `wparam: 0`, `lparam: 250`, `expect_reply: "1"`.
- **Probe Steps (Repeat):**
  Identical sequence for id 253 probed against getter 251.

### 2.3 Distinguishing candidate answers
- **Candidate A (Dedicated Query Mode):**
  - In step 2 (when OFF), `252` returns `0`. In step 3, getter `250` returns `0` (state was preserved).
  - In step 6 (when ON), `252` returns `1`. In step 7, getter `250` returns `1` (state was preserved).
- **Candidate B (Truthy Setter):**
  - In step 2, `252` interprets `-666 != 0` as turning shuffle ON. Step 2 returns `1` (or status). In step 3, getter `250` returns `1`! Shuffle was mutated by `-666`.
- **Candidate C (Ignored / Error / Unsupported):**
  - Step 2 returns `1` (unsupported convention) or `0`, but getter in step 3 remains `0`. When ON (step 6), it returns `1` or `0` irrespective of state.
- **Misleading answer pitfall:** Running this probe when shuffle is already ON would mask a mutating setter, because turning an already-ON toggle ON looks identical to querying an ON toggle (`1` either way). Probing from an established OFF state is mandatory.

### 2.4 Pitfalls
- **Relying solely on the reply of 252:** The reply of a setter might be a status code (e.g., `1`), which could be mistaken for an affirmative query reply. Only reading back the state via the dedicated getter (250/251) proves whether state mutation occurred.

### 2.5 Acceptance criterion and exact artefact
- **Artefact:** Manifest rows for steps `shuff_probe_minus666`, `shuff_readback_after_minus666`, `rep_probe_minus666`, and `rep_readback_after_minus666` in `run-manifest.json`.
- **Acceptance Criterion:** The read-back values on 250/251 establish whether `-666` mutated state (`0 -> 1`) or acted as a query (`0 -> 0`).
- **Harness Readiness:** **Answerable with today's harness.**

---

## 3. IPC 126 Mode 0: Is Stream Info Hz (44100) or kHz (44)?

### 3.1 The exact question and code impact
- **The Question:** When `IPC_GETINFO` (id 126) is sent with `wParam = 0` (sample rate mode) while a stream is actively playing, does it return the sample rate in raw **Hz** (e.g., `44100`) or in **kHz** (e.g., `44`)? What do mode 1 (bitrate) and mode 2 (channels) return during playback?
- **Status in S10:** S10 §3 row 126 and §13.2 state that id 126 is provisional: in stopped state, modes 0, 1, and 2 return `0`. Stream info during playback was never measured because all previous runs were stopped.
- **What an answer changes in our code:**
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L814-L822):
    - Line 817 currently returns `IpcAnswer::Value(self.sample_rate)` (raw Hz, e.g. 44100). If 2.95 returns kHz, line 817 must change to `self.sample_rate / 1000`.
  - In [`crates/rustamp-plughost/src/state_bridge.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/state_bridge.rs#L30-L31,L113):
    - Bridges `sample_rate_hz` from `PlayerState`. If plughost or proto expects kHz, scaling occurs here.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L1160-L1173):
    - Unit test `finding_4_stream_info_playing_sample_rate_fallback` asserts `44_100`. If kHz is returned, it must assert `44`.

### 3.2 The probe
- **Target:** Main window (`class: "Winamp v1.x"`).
- **Window Message:** `WM_USER` (`1024`). `lParam = 126` (`IPC_GETINFO`).
- **Execution Mode:** Synchronous `SendMessageTimeoutW` with `expect_reply: true`.
- **Test Media:** Staged test file from `rustamp-testmedia` (`jingle_44k.mp3`: 44.1 kHz, 128 kbps, stereo 2-channel, duration ~5 seconds).
- **Initial State:** Playing state.
  1. Enqueue and play track via command line or `IPC_STARTPLAY` (id 102).
  2. Settle wait 500ms.
  3. Pre-flight check: Send `msg: 1024`, `wparam: 0`, `lparam: 104` (`IPC_ISPLAYING`), `expect_reply: "1"`.
- **Probe Steps:**
  1. `info_mode0_samplerate`: Send `msg: 1024`, `wparam: 0`, `lparam: 126`, `expect_reply: "44100"` (hypothesis: raw Hz).
  2. `info_mode1_bitrate`: Send `msg: 1024`, `wparam: 1`, `lparam: 126`, `expect_reply: "128"` (hypothesis: kbps).
  3. `info_mode2_channels`: Send `msg: 1024`, `wparam: 2`, `lparam: 126`, `expect_reply: "2"` (hypothesis: stereo channels).

### 3.3 Distinguishing candidate answers
- **Mode 0 (Sample Rate):**
  - *Candidate A (Raw Hz):* Returns `44100`.
  - *Candidate B (kHz):* Returns `44`.
  - *Candidate C (Unsupported / Error):* Returns `0` or `1`.
- **Mode 1 (Bitrate):**
  - *Candidate A (kbps):* Returns `128`.
  - *Candidate B (bps):* Returns `128000`.
- **Mode 2 (Channels):**
  - Returns `2` (stereo) vs `1` (mono).
- **Misleading answer pitfall:** If the probe executes while playback is stopped or has already finished, mode 0 returns `0`. The pre-flight `IPC_ISPLAYING` probe verifying return value `1` is essential to prevent false negatives.

### 3.4 Pitfalls
- **Track end during probe:** If the test track is too short, playback may stop before step 126 executes. Use a track of sufficient length (> 3 s) and probe early in playback.
- **Decoder header latency:** MP3 input plugins parse headers during stream startup. Allow 500ms after `play` before reading id 126.

### 3.5 Acceptance criterion and exact artefact
- **Artefact:** Manifest rows in `run-manifest.json` for `info_mode0_samplerate`, `info_mode1_bitrate`, and `info_mode2_channels`, coupled with `s10_104_isplaying` returning `"1"`.
- **Acceptance Criterion:** Step `info_mode0_samplerate` returns either `44100` or `44` deterministically.
- **Harness Readiness:** **Answerable with today's harness.** Real test media staging and playback automation are already operational in the test VM (validated in M8/M9).

---

## 4. IPC 242: Is "Internet Available" 1 or 0, and Does It Depend on the Network State?

### 4.1 The exact question and code impact
- **The Question:** Does `IPC_INETAVAILABLE` (id 242) return a static constant (always `1`), or does it actively check OS network connectivity (returning `1` when online and `0` when disconnected)?
- **Status in S10:** S10 §3 row 242 and §13.7 state: "provisional — sandbox-networking-dependent; not a fixture property, do not export it as one". Previous runs measured 1 in a sandbox with default networking enabled.
- **What an answer changes in our code:**
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L312-L323):
    - Table definition for row 242 declares `measured: MeasuredReply::Fixed(1)`. If 2.95 dynamically checks connectivity, `MeasuredReply` must be changed from `Fixed(1)` to `Stateful` or dynamic.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L720,L928):
    - Line 720 defaults `inet_available: true`. Line 928 returns `u32::from(self.inet_available)`. If 2.95 is a hardcoded stub returning 1, `inet_available` can be removed or simplified. If it reflects connectivity, the player state bridge must propagate real OS network status.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L1175-L1183):
    - Unit test `finding_4_inet_available_provisional_reply` must be updated to match the settled contract.

### 4.2 The probe
- **Target:** Main window (`class: "Winamp v1.x"`).
- **Window Message:** `WM_USER` (`1024`). `lParam = 242` (`IPC_INETAVAILABLE`), `wParam = 0`.
- **Execution Mode:** Two comparative runs in the test VM:
  - **Run A (Network Connected):**
    - State: Guest network adapter enabled with valid gateway/DNS.
    - Step: `inet_probe_online`: Send `msg: 1024`, `wparam: 0`, `lparam: 242`, `expect_reply: "1"`.
  - **Run B (Network Disconnected):**
    - State: Guest network adapter completely disabled via `Disable-NetAdapter -Name * -Confirm:$false` prior to starting Winamp.
    - Step: `inet_probe_offline`: Send `msg: 1024`, `wparam: 0`, `lparam: 242`, `expect_reply: "0"` (hypothesis: reports offline).

### 4.3 Distinguishing candidate answers
- **Candidate A (Dynamic OS Network Query):**
  - Run A returns `1`.
  - Run B returns `0`.
  - Proves 2.95 queries network state (e.g., via `InternetGetConnectedState`).
- **Candidate B (Static Stub):**
  - Run A returns `1`.
  - Run B returns `1`.
  - Proves 2.95 hardcodes `1` (or only tests presence of Winsock DLLs, which remain loaded).
- **Candidate C (Dial-up Dependent / Unsupported):**
  - Both runs return `0` (or `1` under specific rasapi32 configurations).
- **Misleading answer pitfall:** Disabling an adapter while Windows retains cached NLA (Network Location Awareness) profiles. The guest script must confirm `Get-NetConnectionProfile` returns no active interfaces before launching the reference binary.

### 4.4 Pitfalls
- **Hyper-V management disruption:** Disabling the network adapter inside the guest must not break the broker runner. Because the VM runner operates over Hyper-V PowerShell Direct (VMBus / KVP) and does NOT use TCP/IP network, guest network adapter manipulation is safe and does not interrupt orchestration.

### 4.5 Acceptance criterion and exact artefact
- **Artefact:**
  - Manifest row `inet_probe_online` in `run-manifest.json` from Run A.
  - Manifest row `inet_probe_offline` in `run-manifest.json` from Run B.
  - Transcript lines in `guest-transcript.txt` logging output of `Get-NetAdapter` for each run.
- **Acceptance Criterion:** Comparison between Run A and Run B deterministically shows whether id 242 toggles (`1 -> 0`) or stays constant (`1 -> 1`).
- **Harness Readiness:** **Partially answerable today; needs a small guest provision script.** The capture harness currently does not include a network toggle step. A pre-launch script (`guest/disable-network.ps1`) must be staged in the guest job runner before execution.

---

## 5. IPC 260/281: Window and Menu Handle Returns

### 5.1 The exact question and code impact
- **The Question:**
  1. What exact handles are returned by `IPC_GETWND` (id 260) and `IPC_GET_HMENU` (id 281) across different window lifecycles (open, hidden, minimized, uncreated)?
  2. Does `IPC_GETWND` return `0` (NULL) when a window (e.g. Video, Minibrowser) is closed or disabled, or does the HWND persist?
  3. Does `IPC_GET_HMENU` return the main menu bar handle or a popup context menu handle, and what is its item structure?
  4. **The architectural question:** What is a handle "valid in the plugin's process"? In Winamp 2.95, Winamp was a single 32-bit process, so HWNDs and HMENUs were native to that process. In Rustamp, the player is 64-bit (`rustamp.exe`) and plugins run in an out-of-process 32-bit host (`rustamp-plughost32.exe`). HWNDs in Windows are system-wide handles (32-bit values sign-extended to 64-bit), but cross-process manipulation is strictly limited (e.g., cross-process subclassing `SetWindowLongPtr(GWLP_WNDPROC)` is forbidden by Windows, and `HMENU` handles cannot be directly manipulated or tracked across processes).
- **Status in S10:** S10 §3 rows 260 and 281 record that runs R2/R3 measured non-zero handles (`66262`, `262789`, etc.), but mark them as **shape-only**. S10 §8.1 and §13.8 explicitly state: *"how our host supplies handles valid in the plugin's process is an M3 architecture decision, not a 2.95 fact. Settling: the M3 design decision, then a plugin exercising 260/281 against our host."*
- **What an answer changes in our code:**
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L877-L888,L899-L906):
    - Lines 877–888 (`260 =>`): Returns `self.eq_hwnd`, `self.playlist_hwnd`, etc.
    - Lines 899–906 (`281 =>`): Returns `self.menu_handle`.
    - Currently, all these handles default to `0` in `IpcSurface::new()` ([`ipc.rs:714-719`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L714-L719)). If 260/281 must return valid handles, plughost must either:
      - Option A: Receive 64-bit window HWNDs from `rustamp.exe` over the control channel and cast them to 32-bit HWNDs.
      - Option B: Create local 32-bit dummy/proxy HWNDs in `rustamp-plughost` and forward messages.
      - For 281: Construct a local 32-bit `HMENU` populated with standard command IDs and forward `WM_COMMAND` to the player.
  - In [`crates/rustamp-plughost/src/state_bridge.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/state_bridge.rs#L58-L62,L130-L132):
    - Extend `apply_player_state` or create a new message `Message::WindowHandles` to bridge handles if Option A is chosen.
  - In [`crates/rustamp-plughost/src/ipc.rs`](file:///C:/Users/micro/.local/share/opencode/worktree/26de0e/wave39-s10-probes/crates/rustamp-plughost/src/ipc.rs#L1185-L1199):
    - Unit test `finding_5_window_and_menu_handles_default_to_zero_pending_m3_decision` must be updated once the M3 architecture RFC is approved.

### 5.2 The probe
This item requires a two-part investigation:

#### Part 1: Reference 2.95 Handle Lifecycle Probes (External Harness)
- **Target:** Main window (`class: "Winamp v1.x"`).
- **Window Message:** `WM_USER` (`1024`). `lParam = 260` (wParam: 0=EQ, 1=PE, 2=MB, 3=Video) and `281` (wParam: 0).
- **Probe States:**
  1. `wnd_fresh`: Fresh launch. Probe 260 (0..=3) and 281.
  2. `wnd_hidden_eq`: Close EQ window (`WM_COMMAND` 40036). Re-probe 260 (wparam 0). Does it still return the HWND or 0?
  3. `wnd_hidden_pe`: Close PE window (`WM_COMMAND` 40058). Re-probe 260 (wparam 1).
  4. `wnd_video_toggle`: Open Video window (`WM_COMMAND` 40040). Re-probe 260 (wparam 3).

#### Part 2: Consumer Plugin Behavior & Menu Inspection (In-Process Probe DLL)
- Because external `send-message` cannot inspect window styles, window procedures, or HMENU structure, an in-process probe DLL (`probe_s10.dll`) is loaded into Winamp 2.95:
  1. Calls `SendMessage(main_hwnd, WM_WA_IPC, 0, 281)` to obtain `HMENU`.
  2. Calls `GetMenuItemCount(hmenu)`.
  3. Calls `GetSubMenu(hmenu, 0)` and iterates menu items (`GetMenuItemID`, `GetMenuStringA`).
  4. Calls `GetClassNameA` and `GetWindowLongPtrA` on the HWNDs returned by 260.
  5. Attempts subclassing (`SetWindowLongPtr(GWLP_WNDPROC)`) on the returned HWNDs to see if 2.95 plugins do or can hook them.

### 5.3 Distinguishing candidate answers
- **For 260 (Window Handles):**
  - *Candidate A (Persistent HWND):* Returns non-zero HWND even when window is hidden/closed (window remains allocated in memory).
  - *Candidate B (Zero when Hidden):* Returns `0` when window is closed; non-zero only when visible.
- **For 281 (Menu Handle):**
  - *Candidate A (Main Menu Bar):* Returns an HMENU whose submenus correspond to Main Menu items.
  - *Candidate B (Popup / Options Menu):* Returns an HMENU whose root items are the context menu entries.
- **For Rustamp Architecture:**
  - *Candidate A (Direct 64-bit HWND Cast):* Plugins only send messages (`SendMessageW(hwnd, WM_COMMAND, ...)`) and query dimensions (`GetWindowRect`). Passing `rustamp.exe`'s HWND works directly.
  - *Candidate B (Proxy Window & Menu Emulation):* Plugins call in-process APIs (`TrackPopupMenu`, `SetWindowLongPtr`). Passing cross-process handles causes crashes or silent failures; plughost must create local proxy Win32 objects.

### 5.4 Pitfalls
- **Shape-only numbers:** A 64-bit harness reading a 32-bit handle number over `SendMessageTimeoutW` verifies only that a number was returned. It tells nothing about whether that handle is valid or functional inside an out-of-process helper.
- **Volatile per-boot handles:** HWNDs and HMENUs change on every OS launch. Asserting a hardcoded handle value will fail.

### 5.5 Acceptance criterion and exact artefact
- **Reference Measurement Artefact:**
  - Manifest rows in `run-manifest.json` showing 260 return values in open vs hidden states.
  - Log output from in-process `probe_s10.dll` detailing menu item count, item strings, and HWND class names.
- **Rustamp Architectural Artefact:** An approved M3 design RFC in `docs/rfc/` specifying handle ownership and proxy strategy, followed by an end-to-end integration test in `tests/plughost_e2e.rs`.
- **Harness Readiness:**
  - **Part 1 (External handle queries): Answerable with today's harness.**
  - **Part 2 (Menu inspection & cross-process validity): Not answerable with today's harness.** Requires building an in-process 32-bit probe plugin DLL for Winamp 2.95 and an M3 architecture RFC for Rustamp.

---

## 6. Summary Table

| # | Reply / IPC | Question | Probe | Candidate Answers | Proving Artefact | Harness Readiness |
|---|---|---|---|---|---|---|
| **1** | `122` (Volume)<br>`123` (Pan) | Does a sent setter return the new value, old value, or status code? | Send 122/123 synchronously with `expect_reply` across transitions (`0 -> 100 -> 150` for vol; `0 -> 64 -> -64` for pan). | A: New value (`100`, `150`)<br>B: Old value (`0`, `100`)<br>C: Status (`1` or `0`) | `run-manifest.json` rows for `vol_set_100`, `vol_set_150`, `pan_set_64` | **Ready today** (no harness changes needed) |
| **2** | `252` (Shuffle)<br>`253` (Repeat) | Does `wParam = -666` act as a query or as a truthy mutating setter? | From OFF state, send `-666` to 252/253; read back state via dedicated getters 250/251. | A: Query (state stays `0`)<br>B: Setter (mutates state to `1`)<br>C: Unsupported (returns `1`, state `0`) | `run-manifest.json` rows for probe and subsequent 250/251 read-back | **Ready today** (no harness changes needed) |
| **3** | `126` Mode 0 | Is stream info sample rate returned in raw Hz (44100) or kHz (44) while playing? | Play standard 44.1 kHz track (`jingle_44k.mp3`); verify `104` returns `1`; send `126` mode 0, 1, 2. | A: Raw Hz (`44100`)<br>B: kHz (`44`)<br>C: `0` / unpopulated | `run-manifest.json` row `info_mode0_samplerate` with `s10_104_isplaying = 1` | **Ready today** (media staging & playback operational) |
| **4** | `242` | Does `IPC_INETAVAILABLE` dynamically track network state or return a static `1`? | Comparative VM runs: Run A with network enabled; Run B with network disabled. | A: Dynamic (`1` online, `0` offline)<br>B: Static (`1` online, `1` offline)<br>C: Always `0` | Two `run-manifest.json` runs + `guest-transcript.txt` adapter logs | **Needs small script** (guest network disable script required) |
| **5** | `260` (HWND)<br>`281` (HMENU) | What are the handle lifecycles, and how can an out-of-process host provide valid handles? | Part 1: Probe 260 across open/hidden states.<br>Part 2: 32-bit probe DLL inside 2.95 inspecting HMENU & HWND capabilities. | A: HWND persistent vs zero when hidden<br>B: Direct 64-bit HWND cast vs local 32-bit proxies | Manifest rows for Part 1; probe DLL log & M3 architecture RFC for Part 2 | **Part 1 ready today; Part 2 needs probe DLL & RFC** |

---

## 7. What This Does Not Settle

To maintain clean-room integrity, this measurement plan explicitly does *not* settle:
1. **The out-of-process reverse command channel (Finding 3):**
   Probing 2.95 setter replies (122, 123, 252, 253) does not build the missing reverse command pipe from `rustamp-plughost` to `rustamp.exe`. Even once the return values are measured, propagating setter effects to the 64-bit player engine requires an architectural protocol RFC (RFC 0005 O13 / S10 §2).
2. **In-process pointer arguments and function pointers (S10 §8, §13.3, §13.6):**
   Ids that pass or return pointers in the caller's address space (e.g., id 800 `IPC_GETSADATAFUNC`, id 201 `IPC_GETSKIN`, id 334 `IPC_GETINIFILE`, id 505 `IPC_GET_EMBEDIF`) cannot be validated by external window message probes. They require loaded plugin DLL execution.
3. **Internal Win32 menu message dispatching:**
   Measuring the `HMENU` handle value via id 281 does not resolve how the 64-bit player menu tracks popup interactions initiated by 32-bit plugins. That remains an internal Rustamp GUI and plugin-host architecture design decision.
4. **Any actual measurement values:**
   This document specifies only the probes, hypotheses, candidate discriminations, and acceptance criteria. No values are assumed or committed until dirty-room runs are completed and audited.
