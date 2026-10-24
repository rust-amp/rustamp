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
## Third attempt, 2026-10-24: the status texts, seen (normal mode)

Changes to the harness (`vm-broker/guest/measure-transfer.ps1`): the server
paces the body by **size** (~30 s for any file; the 2026-10-21 run ended the
35 KB transfer in 2.3 s, before the first observation), the first observation
follows 1 s after the launch, and a **PNG of the main window is saved every
0.5 s** (`out\transfer-normal\frames\fNNN.png`, 79 frames over ~32 s, of which
the consecutive frames differ). The status texts are drawn into the client
area, which is why title polling could never see them.

Observed (frames f000-f078, `Winamp v1.x` client, 275x116):

| frames | what the main window shows |
|---|---|
| f000-f005 | the title scroller reads `1. [READING ID3] HTTP://127.0.…` (the URL scrolls); `0 kbps` / `0 kHz`; time `00:00`; no seek bar |
| f010 | `1. [BUFFER: 0%] STREAM.MP3 (0:02)`; the readouts already say `128 kbps` / `44 kHz`; the seek bar appears |
| f020 | the same line scrolling, `[BUFFER: 22%]` |
| f040 | `[BUFFER: 65%]`, the visualisation shows bars |
| f060 | `[BUFFER: 0%]` again with the time at `00:02` (a re-buffer while playing) |
| f078 | `1. STREAM.MP3 (0:02)` (no bracket), readouts blank, stopped |

So: the transfer status is **not a separate display and not in the title bar**;
it is **the main window's scrolling title text**, as a bracketed prefix between
the playlist index and the title: first `[READING ID3]` (before the stream's
format is known, with the URL as the title), then `[BUFFER: n%]` with a percent
that counts up and can fall back to 0%. The two-line transfer display of S19 §2
(`Connecting to host...`, `Sending HTTP request`, ...) was **not** seen in this
window at 2 Hz; those strings are either shorter-lived than 0.5 s or live
elsewhere (the pre-play phase before f000 is not covered: the first frame is
taken ~1 s after the launch).

Open for the next run: capture at >= 10 Hz from process start (a hook before
the launch), and the `cut`/`refused` modes still end the process within ~5 s
with no frames.

Frames and logs stay under `D:\code\rustamp-vm\staging\out\transfer-normal\`
(reference-binary output: dirty-room evidence, not copied into this repo).

## Fourth attempt, 2026-10-24: from the first window at ~9 Hz

The diagnostics now run after the loop and the cadence is ~10 Hz (353 frames in
40 s; `frames\f000` is the first frame in which the main window exists).
Contact sheet of f000-f023 (the scroller crop):

* **f000-f001:** the scroller reads the bare URL: `1. HTTP://127.0.0.1:18080/STREA...` -
  no bracket prefix, the title is the URL itself.
* **f002 onward** (about 0.1-0.2 s later): `1. [READING ID3] HTTP://127.0.0...`,
  unchanged through f023 (~2.4 s), so `[READING ID3]` is the long phase.
* **No `Connecting to host...`, `Sending HTTP request`, `Reading HTTP
  response` or counter text** appears in the scroller in any of the first 24
  frames at 0.1 s resolution. Together with the 2 Hz run above this makes it
  unlikely that S19 §2's two-line texts are drawn in the main window's client
  area during a local, fast connect; they may exist only for slow connects or
  in another place. S19 §2 stays **unmeasured**, with this negative on record.
* The bracketed prefixes seen in the run (`[READING ID3]`, `[BUFFER: n%]`) are
  the stream status the player must reproduce in the scroller.

The rest of the 353 frames are not analysed yet.
