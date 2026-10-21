# Streaming transfer display measurement (S19)

## Method

Attempted live measurement of Winamp 2.95's HTTP streaming transfer display texts and window geometry using the Hyper-V test VM (`rustamp-test`).

**Components built:**
- Slow HTTP server (`HttpListener` in guest) — trickles audio body over ~25 s, modes: `normal` (full), `refused` (TCP reject), `cut` (10% body then close)
- Guest capture script (`docs/notes/vm-broker/guest/measure-transfer.ps1`) — enumerates all visible top-level windows of the player PID once per second, logs class, title, client rect, window rect
- Host runner (`docs/notes/vm-run-measure-transfer.ps1`) — restores `base-provisioned` checkpoint, pins 1024×768 display, stages files, runs three modes via broker

**Blocker:** Winamp 2.95 installation in the guest VM failed. The NSIS installer (`winamp295.exe` / `winamp295_full_noaod.exe`) hangs when launched via scheduled task in the interactive session (15 min timeout). The scheduled task mechanism works for simple GUI apps (Notepad test passed), but the installer never completes.

## Results

No live captures were obtained. The draft at `D:\code\rustamp-sandbox\spec-drafts\S19-transfer-display.draft.md` records only what the spec's string table (S19 §2) provides verbatim, plus the open questions from S19 §6 (items 6 and 15).

## What remains unknown

- Which window(s) show the transfer texts (main window, progress dialog, status area)
- Order of appearance (connecting → request → response → transfer?)
- Arrangement of the two transfer lines (vertical stack, overwrite, horizontal)
- Pixel rectangles of each text in the skin's bitmap font
- Stream title display location (marquee, playlist, title bar — open question 6)
- Visual difference between "total unknown" and "total known" variants

## Re-running

Once Winamp installation is resolved (e.g., by extracting the NSIS installer offline or creating a pre-installed checkpoint), run:

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-measure-transfer.ps1 -Baseline base-provisioned -DisplayPin Set1024
```

Outputs land in `D:\code\rustamp-vm\staging\out\transfer-{normal,refused,cut}\` with `observations.jsonl`, `server-log.txt`, `guest-transcript.txt`, and `summary.json`.

## Spec reference

See `specs/S19-streaming.md` §2 (string facts) and §6 (open questions 6, 15).