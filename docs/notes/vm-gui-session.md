# Ignored winit window tests in the VM's interactive session

Why the window tests needed a second path, and the exact steps from
"orchestrator runs one command" to "proof back on the host". Process only -
nothing here is derived from Winamp source, the SDK or the binary.

## 0. Provisioning correction (2026-10-16)

§1 below assumed the guest holds an auto-logon interactive session. It did
not: `vm-provision-test-host.ps1` wrote `AutoAdminLogon`/`DefaultUserName`/
`DefaultDomainName` but never Winlogon's `DefaultPassword`, without which
Winlogon stops at the logon screen and no session is ever created. The
provision script now takes the guest credential and writes `DefaultPassword`
(plus `AutoLogonCount` removal and password-never-expires), the setup script
passes it and proves the session with the new allow-listed guest script
`vm-broker/guest/assert-interactive-session.ps1` before taking the checkpoint,
and the old `base-provisioned` checkpoint (baked without the password) must
be removed and re-taken. Until that re-take, this document's session
assumption does not hold.

## 1. The finding

The broker's guest scripts run as the S4U user in **session 0**, which has no
desktop. The plugin host's plain Win32 window and the end-to-end test pass
there, but every ignored winit window test **panics inside winit** when run
through the normal runner (`docs/notes/vm-run-tests.ps1`), because winit
needs a desktop. The VM (`rustamp-test`) is provisioned with an **auto-logon
interactive session** (see `vm-provision-test-host.ps1` §1, and §0 above on
the correction that made this true); the code already gates GUI
work on the test VM (`rustamp_app::window::require_test_vm` and the
`rustamp-plughost` copy: staged-tree marker or the machine name), so the four
window tests are ready to run - they just need to be **started in that
session**:

- `eq_window::the_equalizer_window_opens_at_its_docked_size_and_position`
  (client 275 x 116 at (26,145))
- `main_menu::the_main_menu_popup_opens_at_the_measured_client_size`
  (client 323 x 569)
- `playlist_window::the_playlist_window_opens_docked_below_the_main_window`
  (client 275 x 116 at (26,261), spawns the real binary)
- `headless::the_windowed_player_opens_a_window_inside_the_test_vm`
  (spawns the real binary, asserts the device report)

## 2. How the tests get into the interactive session, step by step

1. **Host, unelevated** (`docs/notes/vm-run-gui-session.ps1` §§1-2):
   `cargo test --workspace --all-features --no-run` (build exit checked, test
   binary count printed - a shrunk set must never report green), then the same
   staging as `vm-run-tests.ps1`: test binaries, DLLs, `rustamp.exe`, the
   static-CRT i686 plugin host, CRT DLLs, `crates/`, workspace root files.
2. **Broker, elevated, session 0** (existing ops only, no broker change):
   `restore`/`start` (ejects the install DVD so the guest gets `D:`),
   `copyin` of the staged tree to `C:\rustamp\code\Rustamp`, then `guest` with
   the allow-listed `run-gui-session.ps1`.
3. **Guest, session 0 first** (`vm-broker/guest/run-gui-session.ps1`): `subst
   D: C:\rustamp` so the compile-time paths resolve; discovers the
   interactive session via `explorer.exe` (its `SessionId` plus its owner's
   name from `Win32_Process.GetOwner`), falling back to `query session` for an
   Active console row; refuses if there is no interactive session or it
   resolves to session 0. No hardcoded user name - it follows whoever
   auto-logon was provisioned with.
4. **Guest, interactive session**: the script registers a one-shot Scheduled
   Task `RustampGuiSession` as that user with `-LogonType Interactive`
   (console desktop) and `-RunLevel Limited` (the tests need no elevation;
   Highest would add a UAC question the run cannot answer). The shape follows
   the `register-broker-task.ps1` precedent (action + principal + trigger),
   pointed at the console session instead of S4U. The payload is a fixed local
   path (`C:\rustamp\out\gui-session\inner.ps1`) written by the allow-listed
   script itself - never a broker argument - so the allow-list model holds;
   the only broker-controlled input is the optional test-name filter baked
   into that file. The task starts immediately, the script polls a `done.json`
   sentinel (20-minute timeout), then unregisters the task.
5. **Inner run**: asserts its own session is not 0 and the virtual screen is
   non-empty *before any binary runs*; runs each staged binary as
   `<exe> --ignored <exact test name>`; writes one log per case plus
   `session-proof.json` (session, user, computer, staged-tree marker, screen,
   per-case exit + `test result` summary); writes `done.json`.
6. **Host, unelevated**: broker `copyout` of `C:\rustamp\out\gui-session` to
   `<staging>\out\gui-session` (the existing op; its constraints already fit),
   re-checks the proof shows a non-zero session, prints it with per-test
   results, exits 1 on any failure.

## 3. How the run proves it was in that session

`session-proof.json` carries: the numeric **session id** (must be non-zero
and equal the discovered interactive session), the **user** and **computer**,
the staged-tree **marker** (the same `require_test_vm` signal the tests
gate on), the virtual **screen** size, and per case the exit code, the `test
result` summary line and the log file holding the full output - including the
client size/position lines the tests assert (275 x 116, 323 x 569, (26,145),
(26,261)). Three layers make session 0 fail loudly instead of passing: the
Interactive principal cannot start without the logon; the inner script exits 3
before any binary runs there; the outer wrapper and the host runner both
refuse a proof showing session 0 (or no proof at all).

## 4. What the orchestrator must run

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-gui-session.ps1 [-Filter eq_window] [-Reset]
```

`-Filter` is a substring over the four binary stems / test names (default:
all four). `-Reset` restores `base-provisioned` first, like `vm-run-tests.ps1`.
No broker re-registration is needed: this uses only existing broker ops plus
one new allow-listed guest script, which the broker picks up from the repo
path. Like every broker-adjacent change, the new guest script deserves the
owner's review before it runs in the guest.

## 5. What could not be verified without running it

Nothing here was executed (no broker invocation, no guest script, no test, no
VM launch, no commit). Unconfirmed until the first orchestrator run: that the
guest's Task Scheduler starts an Interactive task on the console desktop with
a full desktop for winit (expected: yes, that is what console logon sessions
are for); the exact `query session` fallback parsing on this guest's locale;
timing (the two spawn cases sleep 3 s each; the 20-minute task timeout is
generous); and that the spawn cases find an audio endpoint in the interactive
session (they fail loudly if not, like in session 0).

## 6. Finding: no app change required

The app needs no change for this path: `ensure_gui_allowed` already permits
window creation inside the test VM (marker or computer name, no override env
needed), the tests already gate on `require_test_vm` before anything
graphical exists, and a task-launched process in the interactive session gets
a desktop like any logon process. If the first run shows otherwise (e.g. the
task's token lacks some desktop right), that becomes a finding against the
app crates and is reported, not worked around here.
