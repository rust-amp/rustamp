# Live session handover — 2026-10-17, late

Written for whoever picks this up next. The project's own status lives in
`docs/HANDOFF.md`; this file is the **session**: what is merged, what is running,
and the exact next steps.

## 1. Where the tree is

- Canonical tree `D:\code\Rustamp`, **clean**, `main` = `4114f61`
  (the deadlock rebuild). **36 merges this evening.**
- Lane worktrees: `C:\Users\micro\.local\share\opencode\worktree\26de0e\<lane>`.
  `powershell -File docs\notes\lanes.ps1` prints the whole table, including the
  canonical tree's cleanliness.
- The VM runner now **names a hung binary** (`[HNG]`, 5-minute bound) instead of
  blocking the VM: `docs\notes\vm-run-tests.ps1`.

## 2. The suite (last full run r36: 79 binaries ran, 11 failed, 2 hung)

Fixed since r36: the 14 golden tests (merge `93a16f9`), the audio suites
(`0a6a95d`: replaygain, resident, dsp_chain, engine), the decoder and metadata
reds (`b9b8bcc`), and **both hangs** (`4114f61` rebuilds the DSP round trip as
lockstep; the runner bounds every binary).

Still open, and the **first thing to do**:
```
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset
```
then read the `[FAIL]` and `[HNG]` lines. Expected remainders: `rustamp_audio`
unit tests (3 — could not be isolated statically, **the VM must name them**),
`rustamp_ui` (2 — the goldens merge should have fixed them), `rustamp_app` (2),
`rustamp_dsp` (1) — Google jobs are on the last two.

## 3. What is running

**agy (Google) jobs do not notify.** Their ids live in
`docs/notes/lane-jobs.md` — that file is the memory. Poll each id, **and check
the lane worktree's newest write time**: a job can say `running` while the lane
stopped hours ago (four did, for 3.5 hours).

| job | lane | task |
|---|---|---|
| `1792270956574926700-678c0b60495d80ea9cf8798f` | `wave65-app-red` | app's two red tests + the `easy_move` wiring (W10 F5) |
| `1792270956574926700-7f0408ce1dd0ad3c463cf432` | `wave65-wake-honest` | make the wake-timeout claim honest (W10 F7) |
| `1792271096501457800-35ca8739b7ccd2f3c537eb9b` | `wave66-dsp-red` | the DSP crate's one red test |
| `1792271096501457800-0c6ffcd99f14ba26be0d6b14` | `wave66-approval-store` | the approval store the loader lacks (P5) |
| `1792271096501457800-5274ea47c223c58922d46860` | `wave66-ignored-staging` | stage what the ignored tests need (W12) |

## 4. Rules this session learned the hard way

1. **A lane may not run tests** → every new test is an unverified claim. Run the
   suite after every batch; seven failures in one batch came from exactly that.
2. **Check the gate before the commit.** A red clippy was merged once because the
   shell printed the result instead of stopping on it.
3. **Create the worktree before launching the lane** (two lanes were launched
   into paths that did not exist).
4. **A report without a worktree diff is not a report** (two lanes died silently;
   one wrote into the canonical tree instead of its worktree).
5. **`es.exe` for every file search**: `C:\Program Files\Everything 1.5a\es.exe`
   (owner's standing instruction, in `AGENTS.md`).
6. **Never wait on a failed source** — relaunch on another one. Source failures
   tonight: Muse (silent), Claude Max (rate limit), Codex/OpenAI
   (subscription-sharing limit), Laguna/Nemotron/Space Bunny/DeepSeek/GLM
   (provider errors), and now the **whole free pool** (`free-models-per-day`).
   Working: **Mistral** (paid), **Muse**, **agy** (4-job cap).

## 5. Open decisions for the owner

1. **RFC 0014** (`docs/rfc/0014-fallible-dsp-stage.md`, merged as *Proposed*): a
   DSP stage must fail loudly with a message. Recommendation: fail-to-stop, own
   error kind, no auto-retry. Needs his nod before a lane implements it — the
   DSP adapter currently works around the hole with a latch.
2. **10 credits** to unlock the free-model pool for the rest of the night.
3. **The Paseo heartbeat** cannot be created from here: the CLI
   (`…\Programs\Paseo\resources\bin\paseo.cmd`) refuses with
   `requireCallerAgentId` because this session is an OpenCode session, not a
   Paseo agent. The ready-to-paste prompt is in `docs/notes/lane-jobs.md`.
4. **The beta package is 72 commits stale.** Rebuild, then smoke it:
   `cargo xtask package --release --out D:\code\rustamp-beta\<new>` followed by
   `powershell -File docs\notes\vm-run-smoke.ps1` (it takes the newest folder).

## 6. Where the beta honestly stands

- `docs/tasks/BETA-what-is-in-it.md` — what the shipped bytes contain, what is
  only in the tree, what is only modelled, and which claims rest on an executed
  proof rather than a compile.
- `docs/tasks/BETA-readiness-2.md` — the twelve old findings, the five things to
  fix first, and what a beta must not ship with.
- `docs/tasks/W12-ignored-inventory.md` — 19 ignored tests; a beta must run the
  8 window tests, the 3 host tests, the 2 loader acceptance tests and
  `real_playback` (the only test on the production WASAPI path).
- `docs/tasks/BETA-smoke.md` — the runner that starts the packaged player, and
  the four defects it found in itself by running.

## 7. Tonight's merges worth knowing about

Engine plays through the decoder resident (M15) and through the DSP resident
(M19, with latency compensation and a latched failure path) · both residents are
shipped in the package (X2) · tray and global hotkeys (model only; the two
`window.rs` seams are still open) · RFC 0012 device-error hook · the loader only
loads verified bytes (P5) · the honest flush contract (A13) · ReplayGain end to
end (E4/A15/A16) · the post-resampler true-peak criterion (A17, flat-topping
gone) · skin cursor fallbacks (J4) · the second real decoder factory (B4) · the
cross-process wake (G4) · display clamping (F4) · the classic DSP plugin bridge
(P6) · the playlist model interactions (S06i4) · both golden namespaces
(`93a16f9`) · the deadlock rebuild (M20) · the two adversarial reviews (W10: ten
claims false; M14: the host framework) · the beta documentation.
