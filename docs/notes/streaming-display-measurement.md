# Streaming transfer display measurement (S19)

## Method

Attempted live measurement of Winamp 2.95's HTTP streaming transfer display texts and window geometry using the Hyper-V test VM (`rustamp-test`).

**Components built:**
- Slow HTTP server (`HttpListener` in guest) — trickles audio body over ~25 s, modes: `normal` (full), `refused` (TCP reject), `cut` (10% body then close)
- Guest capture script (`docs/notes/vm-broker/guest/measure-transfer.ps1`) — enumerates all visible top-level windows of the player PID once per second, logs class, title, client rect, window rect
- Host runner (`docs/notes/vm-run-measure-transfer.ps1`) — restores `base-provisioned` checkpoint, pins 1024×768 display, stages files, runs three modes via broker

**Critical fix (2026-10-21):** The original `measure-transfer.ps1` ran its observation loop in session 0, where no desktop windows are visible. The fixed version follows the `run-presented.ps1` pattern: the HTTP server stays in session 0, but the **player launch and entire observation loop run inside the interactive session** via a one-shot scheduled task (Interactive logon). Session 0 only polls for a done sentinel and refuses loudly if the task landed in session 0.

## Results (2026-10-21, second attempt)

**Normal mode:** 40 observations over 40 s. Three visible windows present from first capture:
- `Winamp v1.x` (main) — 275×116 client, title `1. stream.mp3 - Winamp` → `1. stream.mp3 - Winamp [Stopped]`
- `Winamp PE` (playlist editor) — 275×116 client, constant title `Winamp Playlist Editor`
- `Winamp EQ` (equalizer) — 275×116 client, constant title `Winamp Equalizer`

All three windows stacked vertically, constant geometry. No additional windows appear/disappear during transfer.

**Refused mode:** Winamp process exits within ~5 s, no windows observed.

**Cut mode:** Winamp process exits within ~5 s, no windows observed.

**Server log (normal):** Request received, 35 KB transferred in ~2.3 s (headers at 05:17:32.407, body complete at 05:17:34.672). Transfer completes **during the 5 s startup wait**, before the capture loop begins.

## What the First Attempt Got Wrong

1. **Session 0 observation loop** — the original script ran `EnumWindows` in session 0, which cannot see interactive desktop windows. All three runs produced 40 observations with `WindowCount: 0`. The player process existed but its windows were invisible to session 0.
2. **No interactive-session validation** — the script did not verify it was running in an interactive session, so a "pass" with 0 windows was reported as success.
3. **Winamp setup wizard** — the staged reference copy triggered the first-run setup wizard (`/NOREG` flag added to skip).
4. **Server `SendHeaders()` error** — .NET Framework's `HttpListenerResponse` has no `SendHeaders()` method; headers are sent automatically on first write. Fixed with try/catch and logging.

## What the Second Attempt Produced

- **Window classes over time:** Three classes (`Winamp v1.x`, `Winamp PE`, `Winamp EQ`) visible throughout; no others appear/disappear.
- **Geometry changes:** None — all three windows constant 275×116 client size, fixed positions stacked vertically.
- **Title changes:** Only main window changes: stream title → stream title + `[Stopped]` suffix.
- **S19 §2 status texts:** **Not observed in any window title.** The transfer completed before the capture loop started. Status texts (`Connecting to host...`, `Sending HTTP request`, `Reading HTTP response`, `Retrieving file...`, counters) may appear in the main window's status bar (client area) or too briefly for 1 Hz capture.
- **Open question 6 (stream title location):** **Answered — main window title bar** (`Winamp v1.x`), prefixed with playlist index. Not observed in playlist rows or marquee.
- **Open question 15 (status text sequence):** **Not observed** — capture started post-transfer. Only `[Stopped]` suffix observed.

## What Remains Unknown

- Exact placement of S19 §2 status texts (likely in main window status bar / client area, not title bar)
- Visual arrangement of the two-line transfer display (vertical stack, overwrite, horizontal)
- Pixel rectangles of each text in the skin's bitmap font
- "Total unknown" vs "total known" visual difference (test used known Content-Length)
- Proxy-path texts (not tested)
- Behavior for longer transfers where status texts have time to appear

## Re-running

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-measure-transfer.ps1 -Baseline base-provisioned -DisplayPin Set1024
```

Outputs land in `D:\code\rustamp-vm\staging\out\transfer-{normal,refused,cut}\` with `observations.jsonl`, `server-log.txt`, `inner-transcript.txt`, `guest-transcript.txt`, `summary.json`, `run-provenance.json`.

## Spec Reference

See `specs/S19-streaming.md` §2 (string facts) and §6 (open questions 6, 15). Draft with evidence at `D:\code\rustamp-sandbox\spec-drafts\S19-transfer-display.draft.md`.